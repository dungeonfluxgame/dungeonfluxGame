//! The optional language-model expression step; governing design: adventure-generator section 7.
//!
//! The generator stays pure and synchronous. This step runs after it, on a fresh run only: the
//! pattern adventure is the baseline and the reference for every fact, the storyteller is asked
//! once per group (scenes, then voices and labels) to rewrite each model-written slot from the
//! DM's fact sheet, and each slot is accepted or refused on its own. Text that passes is replayed
//! into the generator through a [`ProseSource`], so the final pack goes through exactly the same
//! assembly, mechanical sentences and loader validation as a pattern adventure. Any failure
//! leaves the pattern adventure unchanged.
use super::super::{
    pack::SceneKey,
    storyteller::{Slot, Storyteller, StorytellerError},
};
use super::{
    FactSheet, ForcedEntries, GenerateError, GenerateOptions, GeneratedAdventure, ProseSource,
    Tables, generate_with_prose, record::ModelExpression,
};
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

/// Overall deadline of the whole step; startup never waits longer for the model.
pub(super) const DEADLINE: Duration = Duration::from_secs(25);
/// Slots before the first scene: the five offer labels and the private clue.
const LEADING_SLOTS: usize = 6;
/// Each scene is expressed as a title, a narration and a spoken line.
const SLOTS_PER_SCENE: usize = 3;
/// The invitation every room narration ends with; the lobby depends on it.
const ROOM_INVITATION: &str = "Share the room with your companions";

/// Words a slot may not use. They are sent as `facts.forbidden`, so the storyteller refuses
/// them and retries with a complaint: rules and outcome words (section 7, validators 3), gore
/// and real-world or modern words (validators 4).
const FORBIDDEN_WORDS: &[&str] = &[
    "hp",
    "hit points",
    "armor class",
    "damage",
    "roll",
    "rolls",
    "dc",
    "attack",
    "attacks",
    "saving throw",
    "level",
    "xp",
    "experience points",
    "kills",
    "killed",
    "dies",
    "slain",
    "wins",
    "loses",
    "succeeds",
    "fails",
    "gore",
    "gory",
    "entrails",
    "viscera",
    "dismember",
    "decapitate",
    "mutilate",
    "okay",
    "guys",
    "awesome",
    "smartphone",
    "internet",
    "email",
];

/// Craft notes for the storyteller; they never relax its fixed rules.
const GUIDANCE: &str = "CRAFT\n\
You are rewriting the plain DRAFT of each requested slot (facts.drafts, same slot name) so it reads like an experienced Dungeon Master speaking at the table. The draft only states the facts; it is not wording to keep. Write fresh sentences with your own imagery, rhythm and phrasing, keeping every fact. Copy a draft sentence only when it is already vivid.\n\
- Present tense, concrete sensory detail (sound, smell, light, touch), short clear sentences. No purple prose, no modern idiom or slang, no rules language, no game numbers.\n\
- Titles are short noun phrases of a few words with no final full stop. Offer labels are button text: what the heroes do, starting with a verb, such as asking, vowing or pressing on.\n\
- Narration speaks to the party as \"you\". A spoken line is only the character's own first-person words: no quotation marks, no stage directions, no narration about them.\n\
- Every speaker has a distinct voice; a mannerism may colour a line (a fussy qualifier, a clipped aside) but never replaces its facts. Let facts.cast (voice, delivery, mannerism) shape word choice, rhythm and what they fuss over; an anxious clerk and a gruff herb gatherer must not sound alike. A speaker sounds the same in every line.\n\
- Do not start two slots the same way or reuse the same image twice. Vary sentence shape.\n\
- Keep every name, place, object and creature exactly as in FACTS, and write every must_mention term verbatim.\n\
- Never write a slot's forbidden terms or hint at them; the secret belongs only in the private clue and the closing lines that list no forbidden terms.\n\
- Do not add events, objects, injuries, outcomes or characters the draft does not have. Heroic fantasy for a living-room table: tense, never gory.\n\
- The room narration keeps its closing invitation to share the room with your companions and bring your heroes into the story.\n\
- Complete sentences that fit the maximum length; shorter is fine.\n";

/// What each scene is, so the model can keep the story in order; the two middle scenes are
/// alternatives, one for each way the party can answer the patron.
const SCENE_ROLES: &[(&str, &str)] = &[
    (
        "room",
        "the lobby on the TV while players join; it ends by inviting them in",
    ),
    (
        "opening",
        "the patron approaches the party and asks for help",
    ),
    (
        "dialogue_seal",
        "if the party asks what is wrong: the patron answers, then the enemy appears",
    ),
    (
        "dialogue_escort",
        "if the party offers to help: the patron accepts, then the enemy appears",
    ),
    (
        "combat",
        "the enemy confronts the party and the fight begins",
    ),
    (
        "victory",
        "after the heroes win: the enemy is beaten and the party catches its breath",
    ),
    ("defeat", "if the heroes fall: the errand fails for now"),
    (
        "inn",
        "the party arrives at the destination and the contact greets them",
    ),
];

/// Why the step left the pattern adventure unchanged.
#[derive(Debug)]
pub(crate) enum Declined {
    Storyteller(StorytellerError),
    Deadline,
    /// The generated slot list is not the shape this step was written for.
    Layout,
    NothingAccepted,
    Replay(GenerateError),
}

impl std::fmt::Display for Declined {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Storyteller(error) => write!(f, "{error}"),
            Self::Deadline => write!(f, "expression deadline of {} s passed", DEADLINE.as_secs()),
            Self::Layout => f.write_str("slot layout not recognised"),
            Self::NothingAccepted => f.write_str("no slot of the model text passed validation"),
            Self::Replay(error) => write!(f, "accepted text did not assemble: {error}"),
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Group {
    Scenes,
    Voices,
}

/// One model-written slot of the pattern adventure.
struct Planned<'a> {
    sheet_index: usize,
    name: String,
    group: Group,
    sheet: &'a FactSheet,
}

fn scene_name(key: SceneKey) -> &'static str {
    match key {
        SceneKey::Room => "room",
        SceneKey::Opening => "opening",
        SceneKey::DialogueSeal => "dialogue_seal",
        SceneKey::DialogueEscort => "dialogue_escort",
        SceneKey::Combat => "combat",
        SceneKey::Inn => "inn",
        SceneKey::Victory => "victory",
        SceneKey::Defeat => "defeat",
    }
}

/// Names every slot the generator expressed, in generation order: five labels and the clue,
/// then a title, narration and line for each scene. Slots with no draft (a silent speaker's
/// line) are not model-written.
fn plan(baseline: &GeneratedAdventure) -> Result<Vec<Planned<'_>>, Declined> {
    let sheets = &baseline.fact_sheet;
    let scenes = &baseline.pack.scenes;
    if sheets.len() != LEADING_SLOTS + SLOTS_PER_SCENE * scenes.len() {
        return Err(Declined::Layout);
    }
    let mut planned = Vec::new();
    for (sheet_index, sheet) in sheets.iter().enumerate() {
        if sheet.baseline.is_empty() {
            continue;
        }
        let (name, group) = match sheet_index.checked_sub(LEADING_SLOTS) {
            None => (sheet.slot.clone(), Group::Voices),
            Some(offset) => {
                let scene = scenes
                    .get(offset / SLOTS_PER_SCENE)
                    .ok_or(Declined::Layout)?;
                let group = if sheet.slot == "line" {
                    Group::Voices
                } else {
                    Group::Scenes
                };
                (format!("{}_{}", scene_name(scene.key), sheet.slot), group)
            }
        };
        planned.push(Planned {
            sheet_index,
            name,
            group,
            sheet,
        });
    }
    Ok(planned)
}

/// The slot's terms as the storyteller takes them: at most eight, each short enough.
fn bounded_terms(terms: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut kept: Vec<String> = Vec::new();
    for term in terms {
        if kept.len() < 8 && term.chars().count() <= 64 && !kept.contains(&term) {
            kept.push(term);
        }
    }
    kept
}

fn slot_spec(planned: &Planned<'_>, secret_terms: &[String]) -> Slot {
    let sheet = planned.sheet;
    let mut mention = sheet.must_mention.clone();
    if sheet.slot == "clue" {
        mention.extend(secret_terms.first().cloned());
    }
    if planned.name == "room_narration" {
        mention.push(ROOM_INVITATION.to_owned());
    }
    let baseline = sheet.baseline.to_lowercase();
    // The clue and the closing beat may carry the secret; elsewhere, only what the pattern text
    // for that very slot already says in public is allowed.
    let forbidden = if sheet.slot == "clue" || sheet.beat == "resolution" {
        Vec::new()
    } else {
        secret_terms
            .iter()
            .filter(|term| !baseline.contains(&term.to_lowercase()))
            .cloned()
            .collect()
    };
    Slot::new(&planned.name, sheet.bounds)
        .mentioning(bounded_terms(mention))
        .forbidding(bounded_terms(forbidden))
}

/// The DM's fact sheet for one group. The secret is told only to the group that writes the
/// private clue and the closing lines.
fn group_facts(baseline: &GeneratedAdventure, planned: &[Planned<'_>], with_secret: bool) -> Value {
    let fact = |key: &str| baseline.facts.get(key).cloned().unwrap_or_default();
    let person = |prefix: &str| {
        json!({
            "name": fact(&format!("{prefix}.first")),
            "full_name": fact(&format!("{prefix}.full")),
            "who": fact(&format!("{prefix}.desc")),
            "voice": fact(&format!("{prefix}.voice")),
            "delivery": fact(&format!("{prefix}.delivery")),
            "mannerism": fact(&format!("{prefix}.mannerism")),
            "pronoun": fact(&format!("{prefix}.sub")),
        })
    };
    let speaks = planned
        .iter()
        .any(|slot| slot.name == "combat_line" && !slot.sheet.baseline.is_empty());
    let drafts: BTreeMap<&str, &str> = planned
        .iter()
        .filter(|slot| {
            with_secret
                || (slot.sheet.slot != "clue"
                    && !(slot.sheet.beat == "resolution" && slot.sheet.slot == "line"))
        })
        .map(|slot| (slot.name.as_str(), slot.sheet.baseline.as_str()))
        .collect();
    let mut facts = json!({
        "tone": "heroic fantasy for a living-room table",
        "location": {
            "name": fact("loc.name"),
            "details": [fact("loc.atmos_a"), fact("loc.atmos_b"), fact("loc.atmos_c")],
        },
        "story": {
            "kind_of_errand": fact("hook.kind"),
            "stake": fact("stake.full"),
            "destination": fact("dest.name"),
            "what_the_enemy_wants_afterward": fact("complication"),
            "thanks": fact("reward.line"),
        },
        "scene_roles": SCENE_ROLES.iter().copied().collect::<BTreeMap<&str, &str>>(),
        "cast": {
            "patron": person("patron"),
            "contact": person("contact"),
            "enemy": {
                "name": fact("ant.short"),
                "speaks": speaks,
                "voice": fact("ant.voice"),
                "delivery": fact("ant.delivery"),
            },
        },
        "forbidden": FORBIDDEN_WORDS,
        "drafts": drafts,
    });
    if with_secret {
        facts["secret"] = json!({
            "told_privately_to_one_hero": fact("secret.fact"),
            "confirmed_at_the_end": fact("secret.confirm"),
        });
    }
    facts
}

/// Section 7 validators that need more than the storyteller's own checks: no digits, no markup
/// or stage directions, no capitalised word that is neither a fact nor sentence-initial.
fn semantic_problem(sheet: &FactSheet, text: &str, known_words: &BTreeSet<String>) -> bool {
    let markup = [
        '*', '_', '#', '`', '<', '>', '[', ']', '{', '}', '\\', '|', '~',
    ];
    if text.contains(markup) || (sheet.slot == "line" && text.contains(['(', ')'])) {
        return true;
    }
    if text.chars().any(|c| c.is_ascii_digit())
        && !sheet.baseline.chars().any(|c| c.is_ascii_digit())
    {
        return true;
    }
    if sheet.slot == "title" || sheet.slot.ends_with("_label") {
        return false;
    }
    let mut initial = true;
    for token in text.split_whitespace() {
        for part in token.split('-') {
            let word = part.trim_matches(|c: char| !c.is_alphanumeric());
            let stem = word
                .strip_suffix("'s")
                .or_else(|| word.strip_suffix('\u{2019}'))
                .unwrap_or(word);
            if !initial
                && stem.chars().count() > 1
                && stem.chars().next().is_some_and(char::is_uppercase)
                && !stem.starts_with("I'")
                && !known_words.contains(&stem.to_lowercase())
            {
                return true;
            }
            initial = false;
        }
        initial = token.ends_with(['.', '!', '?', ':']);
    }
    false
}

fn known_words(baseline: &GeneratedAdventure) -> BTreeSet<String> {
    baseline
        .facts
        .values()
        .chain(baseline.fact_sheet.iter().map(|sheet| &sheet.baseline))
        .flat_map(|text| text.split(|c: char| c.is_whitespace() || c == '-'))
        .map(|word| word.trim_matches(|c: char| !c.is_alphanumeric()))
        .map(|word| word.strip_suffix("'s").unwrap_or(word).to_lowercase())
        .filter(|word| !word.is_empty())
        .collect()
}

/// Curly quotation marks and apostrophes become the plain ones the pattern text uses.
fn plain_punctuation(text: &str) -> String {
    text.chars()
        .map(|character| match character {
            '\u{2018}' | '\u{2019}' => '\'',
            '\u{201c}' | '\u{201d}' => '"',
            other => other,
        })
        .collect()
}

/// Models like to wrap a spoken line in quotation marks; the screen shows the words only.
fn without_outer_quotes(text: &str) -> &str {
    let text = text.trim();
    for (open, close) in [
        ('"', '"'),
        ('\u{201c}', '\u{201d}'),
        ('\u{2018}', '\u{2019}'),
    ] {
        if let Some(inner) = text
            .strip_prefix(open)
            .and_then(|rest| rest.strip_suffix(close))
            && !inner.contains(open)
        {
            return inner.trim();
        }
    }
    text
}

/// Replays accepted text into the generator by position and falls back to the pattern text for
/// any slot whose text the generator's own checks would refuse.
struct AcceptedText {
    texts: Vec<Option<String>>,
    position: Cell<usize>,
    used: RefCell<BTreeSet<usize>>,
}

impl ProseSource for AcceptedText {
    fn express(&self, sheet: &FactSheet) -> Result<String, GenerateError> {
        let index = self.position.get();
        self.position.set(index + 1);
        let lowered = |text: &str| text.to_lowercase();
        if let Some(Some(text)) = self.texts.get(index)
            && text.chars().count() <= sheet.bounds
            && !text.contains(['{', '}'])
            && sheet
                .must_mention
                .iter()
                .all(|term| lowered(text).contains(&lowered(term)))
            && sheet
                .forbidden
                .iter()
                .all(|term| !lowered(text).contains(&lowered(term)))
        {
            self.used.borrow_mut().insert(index);
            return Ok(text.clone());
        }
        Ok(sheet.baseline.clone())
    }

    fn validate(&self, _sheet: &FactSheet, _text: &str) -> bool {
        true
    }
}

async fn ask(
    storyteller: &Storyteller,
    baseline: &GeneratedAdventure,
    planned: &[Planned<'_>],
    group: Group,
) -> Result<Answer, StorytellerError> {
    let secret_terms = &baseline.pack.secret_terms;
    let slots: Vec<Slot> = planned
        .iter()
        .filter(|slot| slot.group == group)
        .map(|slot| slot_spec(slot, secret_terms))
        .collect();
    if slots.is_empty() {
        return Ok(Answer::default());
    }
    let facts = group_facts(baseline, planned, group == Group::Voices);
    let answer = storyteller
        .express_adventure_partial(&facts, &slots, GUIDANCE, DEADLINE)
        .await?;
    Ok(Answer {
        accepted: answer.accepted,
        refused: answer
            .rejected
            .iter()
            .map(|(slot, failure)| (slot.clone(), failure.label()))
            .collect(),
        tokens: answer.tokens,
    })
}

/// One group's accepted text, the reasons for the rest and the tokens spent.
#[derive(Default)]
struct Answer {
    accepted: BTreeMap<String, String>,
    refused: BTreeMap<String, &'static str>,
    tokens: u64,
}

/// Rewrites the pattern adventure's model-written slots through the storyteller and rebuilds the
/// adventure around the text that passed. `Err` means: keep the pattern adventure unchanged.
pub(crate) async fn express(
    tables: &Tables,
    seed: u64,
    options: GenerateOptions,
    forced: &ForcedEntries,
    baseline: &GeneratedAdventure,
    storyteller: &Storyteller,
) -> Result<(GeneratedAdventure, ModelExpression), Declined> {
    let model = storyteller
        .model()
        .ok_or(Declined::Storyteller(StorytellerError::Off))?
        .to_owned();
    let planned = plan(baseline)?;
    let both = tokio::time::timeout(
        DEADLINE,
        futures::future::join(
            ask(storyteller, baseline, &planned, Group::Scenes),
            ask(storyteller, baseline, &planned, Group::Voices),
        ),
    )
    .await
    .map_err(|_| Declined::Deadline)?;
    let mut answers: BTreeMap<String, String> = BTreeMap::new();
    let mut refused: BTreeMap<String, &'static str> = BTreeMap::new();
    let mut tokens = 0_u64;
    let mut failure = None;
    for result in [both.0, both.1] {
        match result {
            Ok(answer) => {
                answers.extend(answer.accepted);
                refused.extend(answer.refused);
                tokens = tokens.saturating_add(answer.tokens);
            }
            Err(error) => failure = Some(error),
        }
    }
    if answers.is_empty() {
        return Err(failure.map_or(Declined::NothingAccepted, Declined::Storyteller));
    }
    let known = known_words(baseline);
    let mut texts: Vec<Option<String>> = vec![None; baseline.fact_sheet.len()];
    for slot in &planned {
        let Some(answer) = answers.get(&slot.name) else {
            continue;
        };
        let text = plain_punctuation(if slot.sheet.slot == "line" {
            without_outer_quotes(answer)
        } else {
            answer.trim()
        });
        let text = text.as_str();
        if semantic_problem(slot.sheet, text, &known) {
            refused.insert(slot.name.clone(), "semantic");
        } else if let Some(entry) = texts.get_mut(slot.sheet_index) {
            *entry = Some(text.to_owned());
        }
    }
    let accepted = AcceptedText {
        texts,
        position: Cell::new(0),
        used: RefCell::new(BTreeSet::new()),
    };
    let rebuilt =
        generate_with_prose(tables, seed, options, forced, &accepted).map_err(Declined::Replay)?;
    let used = accepted.used.into_inner();
    let (model_slots, pattern_slots): (Vec<_>, Vec<_>) = planned
        .iter()
        .partition(|slot| used.contains(&slot.sheet_index));
    if model_slots.is_empty() {
        return Err(Declined::NothingAccepted);
    }
    let names = |slots: &[&Planned<'_>]| -> Vec<String> {
        slots.iter().map(|slot| slot.name.clone()).collect()
    };
    let refused = pattern_slots
        .iter()
        .filter_map(|slot| {
            refused
                .get(&slot.name)
                .map(|reason| format!("{}: {reason}", slot.name))
        })
        .collect();
    Ok((
        rebuilt,
        ModelExpression {
            model,
            model_slots: names(&model_slots),
            pattern_slots: names(&pattern_slots),
            refused,
            tokens,
        },
    ))
}

#[cfg(test)]
#[path = "expression_tests.rs"]
mod tests;
