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
    adventure::dossier::{Dossier, Knowledge, Length, QuestionSubject, ResolvedArchetype, Topic},
    npc_dossier::Withheld,
    social::{Approach, Conversation, Standing},
    speech_text::{sentences, shared_words, words},
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

pub(super) fn current_scene_facts(narration: &str, present: &[(String, String)]) -> Value {
    json!({
        "narration": narration,
        "characters_present": present.iter().map(|(name, role)| {
            json!({"name": name, "role": role})
        }).collect::<Vec<_>>(),
    })
}

/// The owned inputs of one reply, kept by its turn so the facts can be selected again from the
/// reading Jev gives after the hero's line is known. Borrowed parts (dossier, withheld terms,
/// conversation, later lines) live on the turn itself.
pub(super) struct Source {
    pub(super) name: String,
    pub(super) hero: String,
    pub(super) standing: Standing,
    pub(super) recent: Vec<(bool, String)>,
    pub(super) scene_title: String,
    pub(super) scripted_line: String,
    pub(super) words: String,
    pub(super) clue: OwnedClue,
}

/// The owned form of [`Clue`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum OwnedClue {
    Absent,
    Told(String),
    Kept,
}

impl Source {
    /// The borrowed inputs for one selection, over the turn's dossier, withheld terms and
    /// conversation.
    pub(super) fn inputs<'a>(
        &'a self,
        dossier: &'a Dossier,
        withheld: &'a Withheld,
        conversation: &'a Conversation,
        later_lines: &'a [String],
    ) -> Inputs<'a> {
        Inputs {
            dossier,
            name: &self.name,
            hero: &self.hero,
            standing: self.standing,
            recent: &self.recent,
            scene_title: &self.scene_title,
            scripted_line: &self.scripted_line,
            words: &self.words,
            withheld,
            conversation,
            later_lines,
            clue: match &self.clue {
                OwnedClue::Absent => Clue::Absent,
                OwnedClue::Told(text) => Clue::Told(text),
                OwnedClue::Kept => Clue::Kept,
            },
        }
    }
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
            .filter(|sentence| words(sentence).len() >= MINIMUM_STATEMENT_WORDS)
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

/// Who answers the hero's words: Jev's closed reading of each question part, or a neutral reading
/// when Jev did not answer in time. No keyword table reads the words.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReadingSource {
    Jev,
    /// Jev did not answer: no focus and no topic, and the reply is logged as neutral.
    Neutral,
}

/// What the hero's words are about. Built from Jev's per-part [`Focus`] labels, or from the
/// keyword reader as the fallback. Fact selection and the room answer read only this type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AskReading {
    pub(super) source: ReadingSource,
    pub(super) about_you: bool,
    pub(super) place: bool,
    pub(super) danger: bool,
    pub(super) private: bool,
    pub(super) topic: Topic,
    /// An approach-driven topic (courtship, threat, bribe): lines on that topic answer.
    pub(super) directed: Option<Topic>,
    /// An open question that asks what to do next ("anything else?", "what should I do
    /// first?"): the errand answers it. Any other open question is small talk.
    pub(super) task: bool,
    /// The hero's words break with the errand (a claim that something is gone or dead).
    pub(super) claims: bool,
    /// A genuine wh-question about the wider world.
    pub(super) world: bool,
    /// The room question kind, read by Jev. A neutral reading asks no room question.
    pub(super) room: RoomReading,
    /// One Jev focus per question part, in part order. `None` when Jev did not read the line.
    pub(super) focus: Option<Vec<Focus>>,
}

/// The room question kinds the room answer chooses between.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct RoomReading {
    /// Asks the speaker's name or identity.
    pub(super) identity: bool,
    /// Asks about the speaker by name ("what is your name", "who are you").
    pub(super) self_name: bool,
    pub(super) owner: bool,
    pub(super) who_here: bool,
    pub(super) ways_out: bool,
    /// A follow-up about the last named room subject ("does she ...", "what about ...").
    pub(super) follow_up: bool,
    /// Asks who an unnamed or absent person is.
    pub(super) unknown_person: bool,
}

/// One closed label Jev chooses for a question part. Rust keeps the fixed mapping from each
/// label to the reading's flags; Jev decides which label fits the words.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Focus {
    SmallTalk,
    Identity,
    SpeakerPast,
    Motive,
    Evidence,
    Employer,
    Payment,
    DeliveryHistory,
    Condition,
    Place,
    Danger,
    PrivateMatter,
    NextStep,
    ClaimAgainstErrand,
    WorldQuestion,
    Owner,
    WhoIsHere,
    WaysOut,
    UnknownPerson,
    FollowUp,
    Other,
}

/// Each focus with its Jev label and the criterion Jev reads. Labels are the closed choice set.
const FOCI: [(Focus, &str, &str); 21] = [
    (
        Focus::SmallTalk,
        "small_talk",
        "Casual talk, a greeting, or an open question that asks nothing specific. It is answered from the speaker's own opinions.",
    ),
    (
        Focus::Identity,
        "identity",
        "Asks who the speaker is, their name, what to call them, or whether they know someone.",
    ),
    (
        Focus::SpeakerPast,
        "speaker_past",
        "Asks about the speaker's family, home, past, service, work or life.",
    ),
    (
        Focus::Motive,
        "motive",
        "Asks why the speaker wants, is after, follows or does something, or what they gain from it.",
    ),
    (
        Focus::Evidence,
        "evidence",
        "Asks how the speaker knows something, or whether they saw it or have proof.",
    ),
    (
        Focus::Employer,
        "employer",
        "Asks who sent, hired or pays the speaker or the hero.",
    ),
    (
        Focus::Payment,
        "payment",
        "Asks about payment, reward, price, or what was promised.",
    ),
    (
        Focus::DeliveryHistory,
        "delivery_history",
        "Asks whether the speaker or the hero has made this delivery or done this kind of work before.",
    ),
    (
        Focus::Condition,
        "condition",
        "Asks whether someone is hurt, injured, winded or all right.",
    ),
    (
        Focus::Place,
        "place",
        "Asks about this place, the town, the road, the area, or what it is like here.",
    ),
    (
        Focus::Danger,
        "danger",
        "Asks about threats, enemies, an ambush, who is hunting or after someone, risk, or what to watch for.",
    ),
    (
        Focus::PrivateMatter,
        "private_matter",
        "Asks about a confidence or whisper the speaker gave this hero alone, or something hinted between them in private. Choose it only when private_confidence_exists is true.",
    ),
    (
        Focus::NextStep,
        "next_step",
        "Asks what to do next, what else is needed, or what to expect from the errand.",
    ),
    (
        Focus::ClaimAgainstErrand,
        "claim_against_errand",
        "States that something the errand needs is gone, broken, sold, stolen, dead, or otherwise contrary to the errand.",
    ),
    (
        Focus::WorldQuestion,
        "world_question",
        "Asks a genuine question about the wider world, history, rumours, or things the speaker may not know.",
    ),
    (
        Focus::Owner,
        "owner",
        "Asks whose a thing is or who owns it.",
    ),
    (
        Focus::WhoIsHere,
        "who_is_here",
        "Asks who else is here, who is present, or who may be listening.",
    ),
    (
        Focus::WaysOut,
        "ways_out",
        "Asks how to leave, which ways out there are, or where an exit leads.",
    ),
    (
        Focus::UnknownPerson,
        "unknown_person",
        "Asks who an unnamed or absent person is.",
    ),
    (
        Focus::FollowUp,
        "follow_up",
        "A follow-up that refers back to the last named person or thing, such as 'does she', 'what about him', or 'whose is that'.",
    ),
    (
        Focus::Other,
        "other",
        "Anything else: a question about the errand, an object or a person that no other label covers.",
    ),
];

/// The closed label choices Jev reads a question part with.
pub(super) fn focus_choices() -> Vec<(&'static str, &'static str)> {
    FOCI.iter()
        .map(|(_, label, criterion)| (*label, *criterion))
        .collect()
}

/// The focus a Jev label names, if it is one of the closed choices.
pub(super) fn focus_of_label(label: &str) -> Option<Focus> {
    FOCI.iter()
        .find(|(_, candidate, _)| *candidate == label)
        .map(|(focus, _, _)| *focus)
}

impl Focus {
    /// Whether this focus reads a part as asking something. Small talk, a bare statement, and a
    /// claim against the errand ask nothing, so they draw no answer line of their own.
    pub(super) fn asks(self) -> bool {
        !matches!(
            self,
            Self::SmallTalk | Self::Other | Self::ClaimAgainstErrand
        )
    }

    /// The question subject an authored answer is filed under, for the questions that have one.
    pub(super) fn question_subject(self) -> Option<QuestionSubject> {
        match self {
            Self::Identity => Some(QuestionSubject::Identity),
            Self::Evidence => Some(QuestionSubject::Evidence),
            Self::Employer => Some(QuestionSubject::Employer),
            Self::Payment => Some(QuestionSubject::Payment),
            Self::DeliveryHistory => Some(QuestionSubject::DeliveryHistory),
            Self::Condition => Some(QuestionSubject::Condition),
            Self::Motive => Some(QuestionSubject::Motive),
            _ => None,
        }
    }

    /// Sets the reading's flags for this focus. The topic is set separately, once per line.
    fn apply(self, ask: &mut AskReading) {
        match self {
            Self::SmallTalk | Self::Other => {}
            Self::Identity => {
                ask.about_you = true;
                ask.room.identity = true;
                ask.room.self_name = true;
            }
            Self::SpeakerPast | Self::Condition | Self::Evidence | Self::Motive => {
                ask.about_you = true;
            }
            Self::Employer | Self::Danger => ask.danger = true,
            Self::Payment | Self::DeliveryHistory | Self::NextStep => ask.task = true,
            Self::Place => ask.place = true,
            Self::PrivateMatter => ask.private = true,
            Self::ClaimAgainstErrand => ask.claims = true,
            Self::WorldQuestion => ask.world = true,
            Self::Owner => ask.room.owner = true,
            Self::WhoIsHere => ask.room.who_here = true,
            Self::WaysOut => ask.room.ways_out = true,
            Self::UnknownPerson => ask.room.unknown_person = true,
            Self::FollowUp => ask.room.follow_up = true,
        }
    }

    /// The topic that prepared answers and personal lines are filed under, when the focus has one.
    fn topic(self) -> Option<Topic> {
        match self {
            Self::Identity | Self::Evidence | Self::Condition => Some(Topic::WhoAreYou),
            Self::SpeakerPast => Some(Topic::YourPast),
            Self::Employer | Self::Danger => Some(Topic::TheAntagonist),
            Self::Place => Some(Topic::ThePlace),
            Self::Motive
            | Self::Payment
            | Self::DeliveryHistory
            | Self::PrivateMatter
            | Self::NextStep
            | Self::ClaimAgainstErrand
            | Self::WorldQuestion => Some(Topic::WhyThisMatters),
            Self::SmallTalk
            | Self::WhoIsHere
            | Self::Owner
            | Self::WaysOut
            | Self::UnknownPerson
            | Self::FollowUp
            | Self::Other => None,
        }
    }
}

/// The topic an approach names outright (courtship, threat, bribe), if it names one.
pub(super) fn directed_for(approach: Approach) -> Option<Topic> {
    match approach {
        Approach::Flattery | Approach::Flirting => Some(Topic::ComplimentOrFlirt),
        Approach::Intimidation => Some(Topic::Threat),
        Approach::Bribe | Approach::Bargain => Some(Topic::Bribe),
        _ => None,
    }
}

impl AskReading {
    /// The reading Jev gives: the union of its focus flags over the parts, in part order.
    /// The first focus that names a topic sets the line's topic.
    pub(super) fn from_focus(foci: Vec<Focus>) -> Self {
        let mut ask = Self {
            source: ReadingSource::Jev,
            about_you: false,
            place: false,
            danger: false,
            private: false,
            topic: Topic::WhyThisMatters,
            directed: None,
            task: false,
            claims: false,
            world: false,
            room: RoomReading::default(),
            focus: None,
        };
        let mut topic_set = false;
        for focus in &foci {
            focus.apply(&mut ask);
            if let Some(topic) = focus.topic()
                && !topic_set
            {
                ask.topic = topic;
                topic_set = true;
            }
        }
        ask.focus = Some(foci);
        ask
    }

    /// Whether Jev read any part of the line as asking something (see [`Focus::asks`]).
    pub(super) fn asks_something(&self) -> bool {
        self.focus.iter().flatten().any(|focus| focus.asks())
    }

    /// The neutral reading when Jev did not read the line: no focus and no flags. The reply then
    /// draws only on the speaker's own opinions, and the reading is logged as neutral.
    pub(super) fn neutral() -> Self {
        Self {
            source: ReadingSource::Neutral,
            about_you: false,
            place: false,
            danger: false,
            private: false,
            topic: Topic::WhyThisMatters,
            directed: None,
            task: false,
            claims: false,
            world: false,
            room: RoomReading::default(),
            focus: None,
        }
    }

    /// Keeps the approach's own topic for a Jev reading. A neutral reading has no Jev topic to keep.
    pub(super) fn with_approach(mut self, approach: Approach) -> Self {
        if self.source == ReadingSource::Jev {
            self.directed = directed_for(approach);
        }
        self
    }

    /// The topic a fallback answer is filed under: the approach's own topic, else the reading's.
    pub(super) fn fallback_topic(&self, approach: Approach) -> Topic {
        directed_for(approach).unwrap_or(self.topic)
    }
}

/// Whether the hero's words are small talk: an open question (nothing topical asked) that is
/// not asking what to do next. Small talk is answered briefly from the speaker's own life
/// and opinions, never with an errand line.
fn small_talk(ask: &AskReading) -> bool {
    !(ask.about_you || ask.place || ask.danger || ask.private || ask.directed.is_some())
        && !ask.task
}

/// How well a line answers the hero's words; a line that cannot answer them scores zero or less.
fn score(line: &Line, ask: &AskReading, typed: &str, talk: bool) -> i32 {
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

/// The parts of what the hero said, up to three: each sentence, then each `;` clause of it. Only
/// sentence punctuation splits the line. Jev reads each part's focus, so no word list splits or
/// classifies the line.
pub(super) fn question_parts(typed: &str) -> Vec<String> {
    let mut parts = Vec::new();
    for sentence in sentences(typed) {
        parts.extend(sentence.split(';').map(str::to_owned));
    }
    parts
        .into_iter()
        .map(|part| part.trim().trim_end_matches(',').trim().to_owned())
        .filter(|part| !part.is_empty())
        .take(QUESTION_PARTS)
        .collect()
}

/// Whether Jev read part `index` of the hero's line as asking something. Small talk, and a part
/// Jev did not read, draw no answer line of their own.
fn asks_by_focus(ask: &AskReading, index: usize) -> bool {
    ask.focus
        .as_ref()
        .and_then(|foci| foci.get(index))
        .is_some_and(|focus| focus.asks())
}

/// Whether the hero's words break with the errand the speaker can tell: a claim word (burned,
/// dead, nothing) sharing a content word with the errand or danger lines. Asking after the
/// private whisper is not a break.
fn claims_against_errand(inputs: &Inputs<'_>, ask: &AskReading, lines: &[Line]) -> bool {
    if ask.private {
        return false;
    }
    ask.claims
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

fn select<'a>(lines: &'a [Line], inputs: &Inputs<'_>, ask: &AskReading) -> Selection<'a> {
    let fresh: Vec<&Line> = lines.iter().filter(|line| !line.said).collect();
    // Score with the same small-talk decision used for eligibility. Otherwise a question
    // grounded in an errand can score a matching biography word ("letter" / "letters").
    let grounded = question_parts(inputs.words)
        .into_iter()
        .enumerate()
        .filter(|(index, _)| asks_by_focus(ask, *index))
        .any(|(_, part)| {
            fresh.iter().any(|line| {
                !matches!(line.kind, Kind::Yourself) && shared_words(&part, &line.text) > 0
            })
        });
    let talk = small_talk(ask) && !grounded && !ask.world;
    let mut ranked: Vec<(i32, usize, &Line)> = fresh
        .iter()
        .enumerate()
        .map(|(order, line)| (score(line, ask, inputs.words, talk), order, *line))
        .filter(|(points, _, _)| *points > 0)
        .collect();
    ranked.sort_by_key(|(points, order, _)| (std::cmp::Reverse(*points), *order));
    // Small talk draws on the speaker's own life and opinions only: an errand line that
    // happens to share a word never answers it. A question grounded in the errand's own
    // things, or asking after the world, is never small talk.
    let eligible = |line: &Line| !talk || matches!(line.kind, Kind::Yourself);
    // A question with no topic and no shared words is an open one ("what now?"): the best lines
    // answer it. Otherwise every line chosen must itself touch the question.
    let topical = ask.about_you || ask.place || ask.danger || ask.private || ask.directed.is_some();
    let touches = |line: &Line| {
        shared_words(inputs.words, &line.text) > 0
            || matches!(
                (line.kind, ask),
                (Kind::Danger, AskReading { danger: true, .. })
                    | (Kind::Place, AskReading { place: true, .. })
                    | (
                        Kind::Yourself,
                        AskReading {
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
    // line that shares a content word with the part. Only a part Jev read as asking draws a line:
    // scene-setting ("forget the lantern a moment") must not harvest an errand line by a shared
    // word. A part with no line of its own is left for the reply to refuse in character.
    for (index, part) in question_parts(inputs.words).into_iter().enumerate() {
        if answer.len() >= ANSWER_LINES {
            break;
        }
        if !asks_by_focus(ask, index) {
            continue;
        }
        if let Some((_, _, line)) = ranked.iter().find(|(_, _, line)| {
            !taken(&answer, line) && eligible(line) && shared_words(&part, &line.text) > 0
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

/// The curated role snapshot permits general practice answers within its domains, never specific
/// claims about current items, people, events, state, or outcomes. Check every serialized string
/// against this listener's withheld terms before adding it to the model context.
fn public_role_archetype(profile: &ResolvedArchetype, withheld: &Withheld) -> Option<Value> {
    fn leaks(value: &Value, withheld: &Withheld) -> bool {
        match value {
            Value::String(text) => withheld.leaks_secret(text),
            Value::Array(items) => items.iter().any(|item| leaks(item, withheld)),
            Value::Object(fields) => fields.values().any(|item| leaks(item, withheld)),
            _ => false,
        }
    }

    let value = json!({
        "provenance": {
            "library_version": profile.library_version,
            "source_id": profile.source_id,
            "category": profile.category,
        },
        "role": profile.label,
        "instincts": profile.instincts,
        "priorities": profile.priorities,
        "boundaries": profile.boundaries,
        "knowledge_domains": profile.knowledge_domains,
        "speech_habits": profile.speech_habits,
        "typical_actions": profile.typical_actions,
        "experience": profile.experience,
        "competence": profile.competence,
        "temperament": profile.temperament,
        "motive": profile.motive,
        "style": profile.style,
        "scope": "narrative guidance and general practice within listed domains; not evidence of specific facts, mechanics, authority, or current actions",
    });
    (!leaks(&value, withheld)).then_some(value)
}

/// What the model is shown about the NPC and the hero, the lines this reply offers, and
/// every line already said to this hero, uncapped. The offered lines count as said once the
/// reply is recorded and uses them. See the module documentation for how lines are chosen;
/// `withheld` also drops any line that mentions one of its secrets. The facts sent to the
/// model carry only the most recent of the said lines; the full list is for the speaking
/// checks, so nothing already spoken anywhere in the conversation is repeated.
/// The reply facts with a neutral reading. Used when Jev has not read the line.
pub(super) fn facts(inputs: &Inputs<'_>) -> (Value, Vec<Offered>, Vec<String>) {
    facts_for(inputs, &AskReading::neutral())
}

/// The reply facts for one reading of the hero's words: which lines answer, which is
/// volunteered, and the rest of the NPC's context.
pub(super) fn facts_for(
    inputs: &Inputs<'_>,
    ask: &AskReading,
) -> (Value, Vec<Offered>, Vec<String>) {
    let dossier = inputs.dossier;
    let withheld = inputs.withheld;
    let conversation = inputs.conversation;
    let kept = |text: &str| (!withheld.leaks_secret(text)).then(|| text.to_owned());
    let trusted = inputs.standing.trusted(dossier);
    let style = &dossier.speech_style;
    let lines = candidate_lines(inputs);
    let selection = select(&lines, inputs, ask);

    let mut emotional_profile = dossier.emotional_profile();
    emotional_profile.feelings.retain(|feeling| {
        !withheld.leaks_secret(&feeling.text) && !withheld.leaks_secret(&feeling.evidence)
    });
    emotional_profile
        .tells
        .retain(|tell| !withheld.leaks_secret(tell));
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
            "voice": dossier.given.voice.as_deref().and_then(kept),
            "manner": dossier.given.mannerism.as_deref().and_then(kept),
            "delivery": dossier.given.delivery.as_deref().and_then(kept),
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
        "emotional_profile": emotional_profile,
        "recent_exchange": recent,
        "scene": {"title": kept(inputs.scene_title)},
        "player": {
            "hero_name": inputs.hero,
            "addressing": "Use the hero name only when needed; no honorific or pronouns are established.",
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
            "may_say_i_dont_know": selection.answer.is_empty(),
        },
    });
    if let Some(line) = selection.volunteer {
        facts["volunteer"] = json!(line.text);
    }
    if let Some(profile) = dossier
        .archetype_profile
        .as_ref()
        .and_then(|profile| public_role_archetype(profile, withheld))
    {
        facts["role_archetype"] = profile;
    }
    if let Some(line) = selection.nearest {
        facts["nearest_you_know"] = json!(line.text);
    }
    if let Some(line) = courtship {
        facts["courtship"] = json!(line);
    }
    if claims_against_errand(inputs, ask, &lines) {
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

/// Keep the storyteller's fact payload within its byte bound without dropping current scene
/// facts. Oldest remembered/background lines are removed first if the bound is reached.
pub(super) fn trim_background_facts(facts: &mut Value) {
    const MAXIMUM_FACTS_BYTES: usize = 16 * 1024;
    while facts.to_string().len() > MAXIMUM_FACTS_BYTES {
        let mut trimmed = false;
        for key in ["already_said", "recent_exchange"] {
            if let Some(lines) = facts[key].as_array_mut()
                && !lines.is_empty()
            {
                lines.remove(0);
                trimmed = true;
                break;
            }
        }
        if !trimmed
            && let Some(lines) = facts["answer_from"].as_array_mut()
            && !lines.is_empty()
        {
            lines.pop();
            trimmed = true;
        }
        if !trimmed {
            break;
        }
    }
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
    fn reply_facts_offer_the_npcs_real_role_and_do_not_guess_hero_addressing() {
        let dossier = crate::gameplay::npc_dossier_tests::adventure(1)
            .dossiers
            .into_iter()
            .next()
            .unwrap();
        let withheld = Withheld::secrets_only(Vec::new());
        let conversation = Conversation::default();
        let inputs = Inputs {
            dossier: &dossier,
            name: &dossier.name,
            hero: "Brakka",
            standing: Standing {
                attitude: super::super::adventure::dossier::Attitude::Indifferent,
                trust: 0,
            },
            recent: &[],
            scene_title: "At the ferry desk",
            scripted_line: "",
            words: "Who are you?",
            withheld: &withheld,
            conversation: &conversation,
            later_lines: &[],
            clue: Clue::Absent,
        };

        let (facts, _, _) = super::facts(&inputs);

        assert_eq!(facts["npc"]["role"], dossier.role_label);
        assert_ne!(facts["npc"]["role"], dossier.role.label());
        assert!(
            facts["player"]["addressing"]
                .as_str()
                .unwrap()
                .contains("by name")
        );
        assert!(
            !facts["player"]["addressing"]
                .as_str()
                .unwrap()
                .contains("miss")
        );
    }

    #[test]
    fn current_scene_facts_include_display_narration_and_present_cast() {
        let present = vec![("Perrin".to_owned(), "Patron".to_owned())];

        let facts = current_scene_facts(
            "Calla takes the ledger. Perrin lets out a long breath.",
            &present,
        );

        assert_eq!(
            facts["narration"],
            "Calla takes the ledger. Perrin lets out a long breath."
        );
        assert_eq!(facts["characters_present"][0]["name"], "Perrin");
        assert_eq!(facts["characters_present"][0]["role"], "Patron");
    }

    #[test]
    fn payload_trimming_keeps_scene_facts_and_drops_old_background_facts_first() {
        let narration = "Calla takes the ledger.";
        let mut facts = json!({
            "now": {"narration": narration},
            "already_said": ["old".repeat(3_000), "new".repeat(3_000)],
        });

        trim_background_facts(&mut facts);

        assert_eq!(facts["now"]["narration"], narration);
        assert_eq!(facts["already_said"].as_array().map(Vec::len), Some(1));
        assert_eq!(facts["already_said"][0], "new".repeat(3_000));
        assert!(facts.to_string().len() <= 16 * 1024);
    }

    #[test]
    fn a_several_part_line_splits_by_sentence_and_semicolon_in_order() {
        assert_eq!(
            question_parts(
                "Name him. Who was this friend, how did he die, and who is Maren to you?"
            ),
            [
                "Name him.".to_owned(),
                "Who was this friend, how did he die, and who is Maren to you?".to_owned(),
            ],
        );
        assert_eq!(
            question_parts("Who sent you; what did they promise?"),
            [
                "Who sent you".to_owned(),
                "what did they promise?".to_owned()
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

#[cfg(test)]
mod reading_tests {
    use super::*;

    #[test]
    fn every_focus_label_is_a_closed_choice_that_maps_back_to_its_focus() {
        let choices = focus_choices();
        assert_eq!(choices.len(), 21);
        for (label, criterion) in choices {
            assert!(!criterion.trim().is_empty(), "{label} needs a criterion");
            assert!(
                focus_of_label(label).is_some(),
                "{label} must map to a focus"
            );
        }
        assert_eq!(focus_of_label("who_knows"), None);
    }

    #[test]
    fn a_jev_reading_sets_only_the_flags_of_its_focus_and_the_first_topic_wins() {
        let ask = AskReading::from_focus(vec![Focus::Place, Focus::Danger]);
        assert_eq!(ask.source, ReadingSource::Jev);
        assert!(ask.place && ask.danger);
        assert!(!ask.about_you && !ask.private && !ask.task && !ask.claims && !ask.world);
        assert_eq!(ask.topic, Topic::ThePlace);
        assert!(!small_talk(&ask));
    }

    #[test]
    fn small_talk_and_other_are_open_readings_while_a_payment_question_is_errand_talk() {
        assert!(small_talk(&AskReading::from_focus(vec![Focus::SmallTalk])));
        assert!(small_talk(&AskReading::from_focus(vec![Focus::Other])));
        assert!(!small_talk(&AskReading::from_focus(vec![Focus::Payment])));
        assert!(AskReading::from_focus(vec![Focus::Payment]).task);
    }

    #[test]
    fn a_claim_and_a_world_question_are_read_only_from_their_own_focus() {
        let claim = AskReading::from_focus(vec![Focus::ClaimAgainstErrand]);
        assert!(claim.claims && !claim.world);
        let world = AskReading::from_focus(vec![Focus::WorldQuestion]);
        assert!(world.world && !world.claims);
    }

    #[test]
    fn a_neutral_reading_asks_nothing_and_its_words_decide_no_flag() {
        for words in [
            "My book burned.",
            "What are the old wards?",
            "Whose cup is this, and who is here?",
        ] {
            let neutral = AskReading::neutral();
            assert_eq!(neutral.source, ReadingSource::Neutral, "{words}");
            assert!(neutral.focus.is_none());
            assert!(small_talk(&neutral));
            assert!(!neutral.claims && !neutral.world && !neutral.private);
            assert_eq!(neutral.room, RoomReading::default());
        }
    }

    #[test]
    fn a_part_draws_a_line_only_when_jev_read_it_as_asking() {
        let ask = AskReading::from_focus(vec![Focus::SmallTalk, Focus::Payment]);
        assert!(!asks_by_focus(&ask, 0));
        assert!(asks_by_focus(&ask, 1));
        assert!(!asks_by_focus(&ask, 2));
        assert!(!asks_by_focus(&AskReading::neutral(), 0));
    }

    #[test]
    fn the_fallback_topic_follows_the_approach_then_the_jev_topic() {
        let jev = AskReading::from_focus(vec![Focus::SpeakerPast]);
        assert_eq!(jev.fallback_topic(Approach::Question), Topic::YourPast);
        assert_eq!(jev.fallback_topic(Approach::Intimidation), Topic::Threat);
        let neutral = AskReading::neutral();
        assert_eq!(
            neutral.fallback_topic(Approach::Question),
            Topic::WhyThisMatters
        );
        assert_eq!(
            neutral.fallback_topic(Approach::Flirting),
            Topic::ComplimentOrFlirt
        );
    }
}
