//! Choosing the facts of one NPC reply: which of the speaker's lines answer what the hero just
//! said, which one is worth volunteering, and how the rest only shapes the voice. Governing
//! design: adventure-generator section 19 and interaction-engine "Grounded creative speech and
//! listener-safe generation".
//!
//! # Why selection exists
//!
//! A dossier holds far more than any one answer needs: a life story, local sayings, the errand.
//! Handing the model a few of them "to use first" made it say them whether or not they answered
//! the question, so replies filled with autobiography and lore. Here every line has a [`Kind`]:
//!
//! * [`Kind::Errand`] and [`Kind::Danger`] are what the speaker can tell the hero;
//! * [`Kind::Place`] answers only a question about the place or a line the question names;
//! * [`Kind::Yourself`] answers only a question about the speaker.
//!
//! Lines are ranked against the hero's words (shared meaningful words weigh most), a line already
//! said in this conversation is never offered again, and the best two become `answer_from`.
//! From the third exchange the best unsaid errand or danger line is offered as `volunteer`, so
//! the conversation moves. Anything that is not offered still shapes the voice, as `inner_state`
//! and `how_you_speak`, and is never to be said.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::{
    adventure::dossier::{Dossier, Knowledge, Length, Topic},
    npc_dossier::Withheld,
    social::{Conversation, Standing, classify_keywords, topic_of},
    speech_text::{normalised, sentences, shared_words, words},
};

/// The exchange from which a trusted confidence is released, and a line is volunteered.
const RELEASE_FROM_EXCHANGE: u32 = 2;
const VOLUNTEER_FROM_EXCHANGE: u32 = 3;
/// How many lines answer one reply at most: one per part of a several-part question.
const ANSWER_LINES: usize = 3;
/// How many parts of a several-part question the facts list, at most.
const QUESTION_PARTS: usize = 3;
/// A statement shorter than this is an exclamation, not something the speaker knows.
const MINIMUM_STATEMENT_WORDS: usize = 4;
/// How many already-said lines are shown, most recent last.
const SAID_LINES: usize = 8;

/// What the speaker's private whisper is to this hero.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Clue<'a> {
    /// This speaker whispered nothing the hero was not also told publicly.
    Absent,
    /// This speaker whispered this to this hero alone.
    Told(&'a str),
    /// This speaker whispered it to another hero, and this one was not told.
    Kept,
}

/// One line the reply may use, and the key its memory is kept under.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Offered {
    pub(super) key: String,
    pub(super) text: String,
}

/// What the model is shown about the NPC and the hero. See the module documentation; `withheld`
/// also drops any line that mentions one of its secrets.
pub(super) struct Inputs<'a> {
    pub(super) dossier: &'a Dossier,
    /// The name the table hears (the cast name).
    pub(super) name: &'a str,
    pub(super) hero: &'a str,
    pub(super) standing: Standing,
    pub(super) recent: &'a [(bool, String)],
    pub(super) scene_title: &'a str,
    pub(super) scripted_line: &'a str,
    pub(super) words: &'a str,
    pub(super) withheld: &'a Withheld,
    /// What this hero and NPC have already said in the conversation.
    pub(super) conversation: &'a Conversation,
    /// Statements the speaker makes only in scenes after this one: not known yet.
    pub(super) later_lines: &'a [String],
    pub(super) clue: Clue<'a>,
}

/// What a line is for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    /// Who the contact is, where the errand leads, what the speaker says in the scenes.
    Errand,
    /// Rumours, hints and how to stand down: what could go wrong.
    Danger,
    /// Local lore and sayings.
    Place,
    /// The speaker's own past, character and wants.
    Yourself,
}

fn kind_of(source: &str) -> Kind {
    match source {
        "identity_line" | "authored_line" | "errand_line" | "private_clue" | "released_line"
        | "lore_line" => Kind::Errand,
        "rumour_line" | "hint_line" | "stand_down_line" | "world_line" => Kind::Danger,
        "place_line" | "place_line2" => Kind::Place,
        _ => Kind::Yourself,
    }
}

struct Line {
    key: String,
    text: String,
    kind: Kind,
    topic: Topic,
    /// Said already in this conversation or an earlier one.
    said: bool,
}

/// Memory key of the `index`-th line of `source` in a tier; `legacy` is the key older saves used
/// for the whole source.
fn keys(tier: &str, source: &str, index: usize) -> (String, String) {
    (
        format!("{tier}:{source}:{index}"),
        format!("{tier}:{source}"),
    )
}

fn tier_lines<'a>(
    tier: &str,
    items: impl Iterator<Item = &'a Knowledge>,
    inputs: &Inputs<'_>,
    found: &mut Vec<Line>,
) {
    let mut counts: BTreeMap<&'a str, usize> = BTreeMap::new();
    for item in items {
        let index = counts.entry(item.source.as_str()).or_insert(0);
        let (key, legacy) = keys(tier, &item.source, *index);
        *index += 1;
        let known_later = item.source == "authored_line"
            && (inputs.later_lines.contains(&item.text)
                || words(&item.text).len() < MINIMUM_STATEMENT_WORDS);
        if known_later {
            continue;
        }
        let said = inputs
            .conversation
            .voiced
            .iter()
            .any(|voiced| *voiced == key || *voiced == legacy);
        found.push(Line {
            key,
            kind: kind_of(&item.source),
            topic: item.topic,
            text: item.text.clone(),
            said,
        });
    }
}

/// Every line the speaker may say to this hero now.
fn candidate_lines(inputs: &Inputs<'_>) -> Vec<Line> {
    let dossier = inputs.dossier;
    let withheld = inputs.withheld;
    let mut found = Vec::new();
    tier_lines(
        "line",
        withheld.safe_lines(&dossier.knowledge.says_freely),
        inputs,
        &mut found,
    );
    if inputs.standing.trusted(dossier) && inputs.conversation.exchange >= RELEASE_FROM_EXCHANGE {
        tier_lines(
            "trusted",
            withheld.safe_lines(&dossier.knowledge.says_when_trusted),
            inputs,
            &mut found,
        );
    }
    for (index, text) in withheld.released_statements(dossier).enumerate() {
        let key = format!("released:{index}");
        found.push(Line {
            said: inputs.conversation.voiced.contains(&key),
            key,
            text: text.to_owned(),
            kind: kind_of("released_line"),
            topic: Topic::WhyThisMatters,
        });
    }
    if let Clue::Told(clue) = inputs.clue {
        // The whisper's own sentences, minus the narration around them ("draws you close").
        for (index, text) in sentences(clue)
            .into_iter()
            .filter(|sentence| {
                let spoken = words(sentence);
                spoken.len() >= MINIMUM_STATEMENT_WORDS
                    && !spoken
                        .iter()
                        .any(|word| matches!(word.as_str(), "you" | "your" | "us" | "others"))
            })
            .enumerate()
        {
            let key = format!("private:{index}");
            found.push(Line {
                said: inputs.conversation.voiced.contains(&key),
                key,
                text,
                kind: kind_of("private_clue"),
                topic: Topic::WhyThisMatters,
            });
        }
    }
    found
}

/// What the hero's words are about.
struct Ask {
    about_you: bool,
    place: bool,
    danger: bool,
    private: bool,
    topic: Topic,
    /// An approach-driven topic (courtship, threat, bribe): lines on that topic answer.
    directed: Option<Topic>,
    /// An open question that asks what to do next ("anything else?", "what should I do
    /// first?"): the errand answers it. Any other open question is small talk.
    task: bool,
}

fn read_ask(typed: &str) -> Ask {
    let lowered = normalised(typed);
    let has = |phrases: &[&str]| phrases.iter().any(|phrase| lowered.contains(phrase));
    let mapped = topic_of(typed, classify_keywords(typed));
    // Leading-space stems so "son" does not match "person", "war" does not match "warm".
    let directed = match mapped {
        Topic::ComplimentOrFlirt | Topic::Threat | Topic::Bribe => Some(mapped),
        _ if has(&[
            "lover", "flirt", "seduc", "to bed", "kiss", "charm", "marry me",
        ]) =>
        {
            Some(Topic::ComplimentOrFlirt)
        }
        _ => None,
    };
    // Personal life (kin, home, service) is the speaker's past; knowing the cast is who
    // they are. Without this the generic mapping sends family questions to the bond and
    // want lines, which outrank the family lines they resemble.
    let personal = has(&[
        "family", "children", "child", " kid", "kids", " son", "sons", "daughter", "wife",
        "husband", "married", "marry", "partner", "sweetheart", "wedding", "parents", "mother",
        "father", "brother", "sister", " home", "where do you live", "where you live", "house",
        " lodg", " live", " living", "grew up", "where are you from", "how did you end up",
        "your service", " serve", " served", " soldier", " war", " army", " unit", " rank",
        "opinion",
    ]);
    let knowing = has(&[
        "how do you know",
        "do you know",
        "know him",
        "know her",
        "know them",
        "old friend",
        "friends with",
        "your friend",
    ]);
    let topic = match directed {
        Some(topic) => topic,
        None if knowing => Topic::WhoAreYou,
        None if personal => Topic::YourPast,
        None => mapped,
    };
    Ask {
        about_you: has(&[
            "who are you",
            "your name",
            "yourself",
            "about you",
            "your past",
            "your life",
            "your story",
            "your family",
            "family",
            "children",
            "child",
            " kid",
            "kids",
            " son",
            "sons",
            "daughter",
            "wife",
            "husband",
            "married",
            "marry",
            "partner",
            "sweetheart",
            "wedding",
            "parents",
            "mother",
            "father",
            "brother",
            "sister",
            " home",
            "where do you live",
            "where you live",
            "house",
            " lodg",
            " live",
            " living",
            "grew up",
            "where are you from",
            "how did you end up",
            "why do you",
            "what do you want",
            "to you",
            "are you afraid",
            "are you scared",
            "do you fear",
            "what are you hiding",
            "what do you do",
            "your job",
            "your work",
            "your service",
            " serve",
            " served",
            " soldier",
            " war",
            " army",
            " unit",
            " rank",
            "how do you know",
            "do you know",
            "know him",
            "know her",
            "know them",
            "old friend",
            "friends with",
            "your friend",
            "opinion",
            "personally",
            "have you ever",
        ]),
        place: has(&[
            "this place",
            "the place",
            "town",
            "village",
            "around here",
            "the area",
            "this area",
            "local",
            "neighbourhood",
            "the road",
            "this road",
            "the pass",
            "what is it like",
        ]),
        danger: has(&[
            "danger",
            "making trouble",
            "threat",
            "in our way",
            "stop us",
            "watch for",
            "look out",
            "enemy",
            "enemies",
            "behind this",
            "behind all",
            "behind it",
            "who is after",
            "hunting",
            "attack",
            "ambush",
            "bandit",
            "risk",
            "ahead",
            "block",
            "fight",
            "after us",
            "wants it",
            "watching",
            "who sent",
            "refuse",
        ]),
        private: has(&[
            "told me",
            "whisper",
            "quietly",
            "privately",
            "between us",
            "secret",
            "hinted",
            "mentioned",
            "other way",
            "quicker way",
            "you said",
        ]),
        topic,
        directed,
        task: has(&[
            "anything else",
            "what else",
            "tell me more",
            "go on",
            "should",
            "need",
            "expect",
            "next",
            "first",
            "plan",
            "watch for",
            "look out",
            "ahead",
        ]),
    }
}

/// Whether the hero's words are small talk: an open question (nothing topical asked) that is
/// not asking what to do next. Small talk is answered briefly from the speaker's own life
/// and opinions, never with an errand line.
fn small_talk(ask: &Ask) -> bool {
    !(ask.about_you || ask.place || ask.danger || ask.private || ask.directed.is_some())
        && !ask.task
}

/// Whether the hero's words ask after the world with a wh-word ("what are the old wards?").
/// A genuine question refuses in character when nothing answers it: it never takes small
/// talk ("ever seen a dragon?" asks no wh-word) or padding.
fn world_question(typed: &str) -> bool {
    words(typed).iter().any(|word| {
        matches!(
            word.as_str(),
            "what" | "who" | "whom" | "whose" | "where" | "when" | "why" | "how" | "which"
        )
    })
}

/// How well a line answers the hero's words; a line that cannot answer them scores zero or less.
fn score(line: &Line, ask: &Ask, typed: &str) -> i32 {
    let talk = small_talk(ask);
    // Small talk asks nothing topical, so shared words are incidental ("seen", "ever") and
    // must not let a biography line outrank an opinion.
    let shared = if talk {
        0
    } else {
        i32::try_from(shared_words(typed, &line.text)).unwrap_or(0)
    };
    let kind = match line.kind {
        Kind::Errand if ask.about_you && line.key.contains(":identity_line:") => 6,
        Kind::Errand if line.key.contains(":authored_line:") => 4,
        Kind::Errand => 3,
        Kind::Danger if ask.danger => 6,
        Kind::Danger => 2,
        Kind::Place if ask.place && !ask.danger => 6,
        Kind::Place if ask.place => 2,
        Kind::Place if shared > 1 => 0,
        Kind::Place => -100,
        // An enemy asked what it wants or why gives its motive before its biography.
        Kind::Yourself if ask.about_you && line.key.contains(":motive_line:") => 7,
        Kind::Yourself if ask.about_you => 4,
        // Small talk draws on opinions first, then the rest of the speaker's own life.
        Kind::Yourself if talk && line.key.contains(":opinion_line:") => 6,
        Kind::Yourself if talk => 3,
        Kind::Yourself => -100,
    };
    // A question about the speaker is answered by the line on that very topic ("who are you"
    // by the speaker's own introduction, "your past" by their history).
    let topic = if ask.about_you && line.topic == ask.topic {
        20
    } else {
        0
    };
    let private = if ask.private && line.key.starts_with("private:") {
        8
    } else {
        0
    };
    shared * 8 + kind + topic + private
}

/// Words that mark a question, for splitting "who ..., how ..., and who ..." into parts.
const QUESTION_WORDS: [&str; 12] = [
    "who", "what", "where", "when", "why", "how", "which", "whose", "name", "tell", "explain",
    "describe",
];

/// Words by which a hero's claim breaks with the errand: the book burned, the friend dead.
const BREAK_WORDS: [&str; 14] = [
    "burned",
    "burnt",
    "destroyed",
    "lost",
    "sold",
    "stole",
    "stolen",
    "dead",
    "died",
    "killed",
    "took",
    "taken",
    "nothing",
    "never",
];

/// Splits `chunk` on `separator` when both sides still ask something (each side keeps an
/// interrogative or at least three words); otherwise the chunk stays whole.
fn split_clause(chunk: &str, separator: &str) -> Vec<String> {
    let split: Vec<String> = chunk.split(separator).map(str::to_owned).collect();
    if split.len() < 2 {
        return split;
    }
    let asks = |text: &str| {
        let spoken = words(text);
        spoken
            .iter()
            .any(|word| QUESTION_WORDS.contains(&word.as_str()))
            || spoken.len() >= 3
    };
    if split.iter().all(|part| asks(part)) {
        split
    } else {
        vec![chunk.to_owned()]
    }
}

/// One clause of a chunk that asks more than one thing ("who ..., how ..., and who ...").
fn split_clauses(chunk: &str) -> Vec<String> {
    let spoken = words(chunk);
    let interrogatives = spoken
        .iter()
        .filter(|word| QUESTION_WORDS.contains(&word.as_str()))
        .count();
    if interrogatives < 2 {
        return vec![chunk.to_owned()];
    }
    let mut parts = vec![chunk.to_owned()];
    for separator in [", and ", " and ", ", "] {
        let mut split = Vec::new();
        for part in &parts {
            split.extend(split_clause(part, separator));
        }
        parts = split;
    }
    parts
}

/// Whether the part asks something: a question mark or a question word. Scene-setting
/// ("forget the lantern a moment") does not, and draws no answer line of its own.
fn asks(part: &str) -> bool {
    part.contains('?')
        || words(part)
            .iter()
            .any(|word| QUESTION_WORDS.contains(&word.as_str()))
}

/// The parts of what the hero said, up to three: sentences first, then the clauses of a
/// sentence that asks more than one thing. The reply answers each part in order.
pub(super) fn question_parts(typed: &str) -> Vec<String> {
    let mut parts = Vec::new();
    for sentence in sentences(typed) {
        for chunk in sentence.split(';') {
            parts.extend(split_clauses(chunk));
        }
    }
    parts
        .into_iter()
        .map(|part| part.trim().trim_end_matches(',').trim().to_owned())
        .filter(|part| !part.is_empty())
        .take(QUESTION_PARTS)
        .collect()
}

/// Whether the hero's words break with the errand the speaker can tell: a claim word (burned,
/// dead, nothing) sharing a content word with the errand or danger lines. Asking after the
/// private whisper is not a break.
fn claims_against_errand(inputs: &Inputs<'_>, ask: &Ask, lines: &[Line]) -> bool {
    if ask.private {
        return false;
    }
    let spoken = words(inputs.words);
    let breaks = BREAK_WORDS
        .iter()
        .any(|word| spoken.iter().any(|spoken| spoken == word));
    breaks
        && lines.iter().any(|line| {
            matches!(line.kind, Kind::Errand | Kind::Danger)
                && shared_words(inputs.words, &line.text) > 0
        })
}

/// The lines chosen for one reply.
struct Selection<'a> {
    answer: Vec<&'a Line>,
    volunteer: Option<&'a Line>,
    nearest: Option<&'a Line>,
}

fn select<'a>(lines: &'a [Line], inputs: &Inputs<'_>, ask: &Ask) -> Selection<'a> {
    let fresh: Vec<&Line> = lines.iter().filter(|line| !line.said).collect();
    let mut ranked: Vec<(i32, usize, &Line)> = fresh
        .iter()
        .enumerate()
        .map(|(order, line)| (score(line, ask, inputs.words), order, *line))
        .filter(|(points, _, _)| *points > 0)
        .collect();
    ranked.sort_by_key(|(points, order, _)| (std::cmp::Reverse(*points), *order));
    // Small talk draws on the speaker's own life and opinions only: an errand line that
    // happens to share a word never answers it. A question grounded in the errand's own
    // things, or asking after the world, is never small talk.
    let grounded = fresh.iter().any(|line| {
        !matches!(line.kind, Kind::Yourself)
            && shared_words(inputs.words, &line.text) > 0
    });
    let talk = small_talk(ask) && !grounded && !world_question(inputs.words);
    let eligible = |line: &Line| !talk || matches!(line.kind, Kind::Yourself);
    // A question with no topic and no shared words is an open one ("what now?"): the best lines
    // answer it. Otherwise every line chosen must itself touch the question.
    let topical = ask.about_you || ask.place || ask.danger || ask.private || ask.directed.is_some();
    let touches = |line: &Line| {
        shared_words(inputs.words, &line.text) > 0
            || matches!(
                (line.kind, ask),
                (Kind::Danger, Ask { danger: true, .. })
                    | (Kind::Place, Ask { place: true, .. })
                    | (
                        Kind::Yourself,
                        Ask {
                            about_you: true,
                            ..
                        }
                    )
            )
            || (ask.private && line.key.starts_with("private:"))
            || (ask.about_you && line.topic == ask.topic)
            || ask.directed.is_some_and(|topic| line.topic == topic)
    };
    let open = !topical
        && ranked
            .first()
            .is_some_and(|(_, _, line)| shared_words(inputs.words, &line.text) == 0);
    let taken = |answer: &[&Line], line: &Line| answer.iter().any(|taken| taken.key == line.key);
    let mut answer: Vec<&Line> = Vec::new();
    // One line per part of a several-part question, in the parts' order: the best-ranked fresh
    // line that shares a content word with the part. Only a part that asks (a question mark
    // or a question word) draws a line: scene-setting ("forget the lantern a moment") must
    // not harvest an errand line by a shared word. A part with no line of its own is left
    // for the reply to refuse in character.
    for part in question_parts(inputs.words) {
        if answer.len() >= ANSWER_LINES {
            break;
        }
        if !asks(&part) {
            continue;
        }
        if let Some((_, _, line)) = ranked.iter().find(|(_, _, line)| {
            !taken(&answer, line)
                && eligible(line)
                && shared_words(&part, &line.text) > 0
        }) {
            answer.push(line);
        }
    }
    // The answer fills from lines that touch the question; an open question also takes a
    // grounded line (it shares a word) or, when asking what to do next, the best lines.
    // Small talk fills from opinions. Nothing else is ever bolted on: a genuine question
    // with no answering line stays empty for the in-character refusal.
    let fills = |line: &Line| {
        eligible(line)
            && (talk
                || touches(line)
                || (open && ask.task)
                || (open && shared_words(inputs.words, &line.text) > 0))
    };
    for (_, _, line) in &ranked {
        if answer.len() >= ANSWER_LINES {
            break;
        }
        if !taken(&answer, line) && fills(line) {
            answer.push(line);
        }
    }
    // An unvoiced line is volunteered only when it answers the question (small talk gets
    // none), or when the hero asks an open "what next": never padding bolted onto an answer.
    let volunteer = (inputs.conversation.exchange >= VOLUNTEER_FROM_EXCHANGE)
        .then(|| best_to_tell(&fresh, &answer))
        .flatten()
        .filter(|line| !talk && ((open && ask.task) || touches(line)));
    let nearest = answer
        .is_empty()
        .then(|| best_to_tell(&fresh, &[]))
        .flatten();
    Selection {
        answer,
        volunteer,
        nearest,
    }
}

/// The unsaid errand or danger line most worth telling that is not already in `except`. What the
/// speaker knows of the errand and a practical hint outrank a bare rumour; the earlier line wins
/// a tie.
fn best_to_tell<'a>(fresh: &[&'a Line], except: &[&Line]) -> Option<&'a Line> {
    let mut best: Option<(u8, &'a Line)> = None;
    for line in fresh.iter().copied() {
        let weight = match line.kind {
            Kind::Errand if line.key.contains(":errand_line:") => 5,
            Kind::Danger if line.key.contains(":hint_line:") => 4,
            Kind::Errand => 3,
            Kind::Danger if line.key.contains(":stand_down_line:") => 3,
            Kind::Danger => 1,
            _ => continue,
        };
        let chosen = except.iter().any(|taken| taken.key == line.key);
        if !chosen && best.is_none_or(|(known, _)| weight > known) {
            best = Some((weight, line));
        }
    }
    best.map(|(_, line)| line)
}

fn length_label(length: Length) -> &'static str {
    match length {
        Length::Short => "short and plain",
        Length::Medium => "medium",
        Length::Long => "long and winding",
    }
}

/// The identity line about someone else ("Tomas is a watch sergeant at the Waystation"): who
/// would know more.
fn who_would_know(dossier: &Dossier, withheld: &Withheld) -> Option<String> {
    withheld
        .safe_lines(&dossier.knowledge.says_freely)
        .find(|item| item.source == "identity_line" && !item.text.starts_with("I am"))
        .map(|item| item.text.clone())
}

/// What the model is shown about the NPC and the hero, the lines this reply offers, and
/// every line already said to this hero, uncapped. The offered lines count as said once the
/// reply is recorded and uses them. See the module documentation for how lines are chosen;
/// `withheld` also drops any line that mentions one of its secrets. The facts sent to the
/// model carry only the most recent of the said lines; the full list is for the speaking
/// checks, so nothing already spoken anywhere in the conversation is repeated.
pub(super) fn facts(inputs: &Inputs<'_>) -> (Value, Vec<Offered>, Vec<String>) {
    let dossier = inputs.dossier;
    let withheld = inputs.withheld;
    let conversation = inputs.conversation;
    let kept = |text: &str| (!withheld.leaks_secret(text)).then(|| text.to_owned());
    let trusted = inputs.standing.trusted(dossier);
    let style = &dossier.speech_style;
    let ask = read_ask(inputs.words);
    let lines = candidate_lines(inputs);
    let selection = select(&lines, inputs, &ask);

    let temperament = &dossier.temperament;
    let reactions: serde_json::Map<String, Value> = [
        ("flattery", &temperament.flattery.text),
        ("flirting", &temperament.flirting.text),
        ("threat", &temperament.threats.text),
        ("lie", &temperament.lies.text),
        ("bribe", &temperament.bribes.text),
    ]
    .into_iter()
    .filter(|(_, text)| !withheld.leaks_secret(text))
    .map(|(name, text)| (name.to_owned(), json!(text)))
    .collect();
    let recent: Vec<Value> = inputs
        .recent
        .iter()
        .filter(|(_, text)| !withheld.leaks_secret(text))
        .map(|(hero, text)| json!({"from": if *hero { inputs.hero } else { inputs.name }, "said": text}))
        .collect();
    // Menu choice labels are never shown to the model: a reply that names them reads the
    // phone's menu aloud instead of answering in character. The facts carry an empty list so
    // the prompt cannot reach for one.
    let choices: Vec<&str> = Vec::new();

    let mut inner = serde_json::Map::new();
    if let Some(want) = kept(&dossier.want.text) {
        inner.insert("want".to_owned(), json!(want));
    }
    if trusted && let Some(fear) = kept(&dossier.fear.text) {
        inner.insert("fear".to_owned(), json!(fear));
    }

    // The standing answer to romance: who (if anyone) holds this heart, in this NPC's own
    // words. Refusals come from it, the same every time; the model never invents a partner.
    let courtship = withheld
        .safe_lines(&dossier.knowledge.says_freely)
        .find(|item| item.source == "courtship_line")
        .map(|item| item.text.clone());
    let mut facts = json!({
        "npc": {
            "name": inputs.name,
            "role": dossier.role_label,
            "voice": dossier.given.voice,
            "manner": dossier.given.mannerism,
            "delivery": dossier.given.delivery,
        },
        "how_you_speak": {
            "sentences": length_label(style.length),
            "vocabulary": style.vocabulary,
            "temperament": dossier.given.tags,
        },
        "inner_state": inner,
        "attitude_toward_hero": inputs.standing.attitude.label(),
        "hero": inputs.hero,
        "answer_from": selection.answer.iter().map(|line| line.text.clone()).collect::<Vec<_>>(),
        "reactions": reactions,
        "recent_exchange": recent,
        "scene": {"title": kept(inputs.scene_title)},
        "player": {
            "hero_name": inputs.hero,
            "exact_words": withheld.redact(inputs.words),
            // The hero's questions, up to three, in order: the reply answers each in turn.
            // Part of the player's own speech, redacted like it, so untrusted words live
            // nowhere else in the prompt.
            "question_parts": question_parts(inputs.words)
                .iter()
                .map(|part| withheld.redact(part))
                .collect::<Vec<_>>(),
        },
        "offered_choices": choices,
        "listener_knowledge": "Say only what these facts allow; your own confidences, when \
    listed, are yours to share.",
        "conversation": {
            "exchange": conversation.exchange,
            "stage": stage_of(conversation.exchange),
            "last_reply_asked": conversation.last_asked,
            "may_say_i_dont_know": selection.answer.is_empty() && !conversation.admitted_ignorance,
        },
    });
    if let Some(line) = selection.volunteer {
        facts["volunteer"] = json!(line.text);
    }
    if let Some(line) = selection.nearest {
        facts["nearest_you_know"] = json!(line.text);
    }
    if let Some(line) = courtship {
        facts["courtship"] = json!(line);
    }
    if claims_against_errand(inputs, &ask, &lines) {
        facts["hero_claims_against_errand"] = json!(true);
    }
    if selection.answer.is_empty()
        && let Some(who) = who_would_know(dossier, withheld)
    {
        facts["who_would_know"] = json!(who);
    }
    match inputs.clue {
        Clue::Told(clue) => facts["you_told_this_hero_in_private"] = json!(clue),
        Clue::Kept => {
            facts["private_matter"] = json!(
                "You confided something to another hero in private. This hero was not told, so you \
will not say what it was, what it was about, or that it holds any detail."
            );
        }
        Clue::Absent => {}
    }
    let said: Vec<String> = lines
        .iter()
        .filter(|line| line.said)
        .map(|line| line.text.clone())
        .collect();
    if !said.is_empty() {
        let from = said.len().saturating_sub(SAID_LINES);
        facts["already_said"] = json!(&said[from..]);
    }
    // What the NPC said as the scene began matters only until the conversation has started;
    // after that it just gets repeated.
    if inputs.recent.is_empty()
        && !inputs.scripted_line.is_empty()
        && let Some(opening) = kept(inputs.scripted_line)
    {
        facts["your_opening_words"] = json!(opening);
    }
    let offered = selection
        .answer
        .iter()
        .copied()
        .chain(selection.volunteer)
        .chain(selection.nearest)
        .map(|line| Offered {
            key: line.key.clone(),
            text: line.text.clone(),
        })
        .collect();
    (facts, offered, said)
}

fn stage_of(exchange: u32) -> &'static str {
    match exchange {
        0..=2 => "early",
        3..=4 => "middle",
        _ => "late",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_several_part_question_splits_into_parts_in_order_capped_at_three() {
        assert_eq!(
            question_parts(
                "Name him. Who was this friend, how did he die, and who is Maren to you?"
            ),
            [
                "Name him.".to_owned(),
                "Who was this friend".to_owned(),
                "how did he die".to_owned(),
            ],
        );
    }

    #[test]
    fn a_single_question_stays_whole() {
        assert_eq!(
            question_parts("Why do you need two strangers?"),
            vec!["Why do you need two strangers?".to_owned()]
        );
        assert_eq!(
            question_parts("Hand me the book now, old man, or I take it from you."),
            vec!["Hand me the book now, old man, or I take it from you.".to_owned()]
        );
    }

    #[test]
    fn at_most_three_parts_are_listed() {
        assert_eq!(
            question_parts("Who sent you? What did they promise? When will they come? Why?").len(),
            QUESTION_PARTS
        );
    }
}
