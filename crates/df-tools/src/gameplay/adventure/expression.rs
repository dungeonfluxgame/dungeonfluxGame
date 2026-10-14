//! The optional language-model expression step; governing design: adventure-generator section 7.
//!
//! The generator stays pure and synchronous. This step runs after it, on a fresh run only: the
//! pattern adventure is the baseline and the reference for every fact, the storyteller is asked
//! once per group (scenes, then voices and labels) to rewrite each model-written slot from the
//! DM's fact sheet, and each slot is accepted or refused on its own. A lint over all accepted
//! slots together then asks once more for just the slots that read badly. Text that passes is
//! replayed into the generator through a [`ProseSource`], so the final pack goes through
//! exactly the same assembly, rules sentences and loader validation as a pattern adventure. Any
//! failure leaves the pattern adventure unchanged. Rules sentences are never sent or written:
//! they live in each scene's `rules_note`.
use super::super::{
    pack::SceneKey,
    startup_configuration::ContentRating,
    storyteller::{
        RESTRAINED_GORE, RESTRAINED_PROFANITY, Slot, Storyteller, StorytellerError, age_group,
        contains_whole_word, mature_themes, sexual_content_allowed,
    },
};
use super::{
    FactSheet, ForcedEntries, GenerateError, GenerateOptions, GeneratedAdventure, ProseSource,
    Tables, generate::tone_of, generate_with_prose, record::ModelExpression,
};
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

/// Overall budget of the whole step; startup never waits longer for the model.
/// Kept only as the outer guard: each slot group runs under its own shorter
/// deadline, so one slow group can no longer discard the text that did arrive.
pub(super) const DEADLINE: Duration = Duration::from_secs(60);
/// Deadline of one slot-group request attempt. Groups are small, so a healthy
/// answer arrives well inside this; a miss is retried once (see `ask_group`).
const GROUP_DEADLINE: Duration = Duration::from_secs(20);
/// Slots per model request. Two whole-adventure requests routinely missed the
/// old single 25 s deadline; small groups answer faster and fail independently.
const GROUP_SLOTS: usize = 6;
/// How many slot groups are in flight at once.
const GROUP_CONCURRENCY: usize = 4;
/// The second ask is skipped when less than this is left of the budget.
const REASK_MINIMUM: Duration = Duration::from_secs(5);
/// Slots before the first scene: the five offer labels and the private clue.
const LEADING_SLOTS: usize = 6;
/// Each scene is expressed as a title, a narration and a spoken line.
const SLOTS_PER_SCENE: usize = 3;
/// Spoken or read aloud: no sentence may run longer than this many words.
const MAXIMUM_SENTENCE_WORDS: usize = 28;
/// The invitation sentence the pattern room narration ends with; it is an instruction to the
/// players, not narration, so the model never sees it.
const ROOM_INVITATION: &str =
    "Share the room with your companions, then bring your heroes into the story.";

/// Rules and outcome words a slot may not use in either rating (section 7, validators 3), and
/// the rules wording that belongs in `rules_note`. They are sent as `facts.forbidden`, so the
/// storyteller refuses them and retries with a complaint.
const FORBIDDEN_RULES_WORDS: &[&str] = &[
    "hp",
    "hit point",
    "hit points",
    "armor class",
    "ac",
    "d20",
    "damage",
    "short rest",
    "prone",
    "feet",
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
    "suddenly",
];

/// Real-world and modern words, refused in either rating.
const FORBIDDEN_MODERN_WORDS: &[&str] =
    &["okay", "guys", "awesome", "smartphone", "internet", "email"];

/// Every word a slot may not use for `rating`. A restrained table also refuses gore and strong
/// language (section 7, validators 4); a mature one allows both.
fn forbidden_words(rating: ContentRating) -> Vec<&'static str> {
    let restrained = rating == ContentRating::Restrained;
    let mut words = FORBIDDEN_RULES_WORDS.to_vec();
    if restrained {
        words.extend(RESTRAINED_GORE);
    }
    words.extend(FORBIDDEN_MODERN_WORDS);
    if restrained {
        words.extend(RESTRAINED_PROFANITY);
    }
    words
}

/// Stock phrases that repeat from game to game; any slot using one is asked again.
const STOCK_PHRASES: &[&str] = &[
    "now, then",
    "you were in time",
    "you stood your ground",
    "something is about to happen",
    "share the room with your companions",
    "bring your heroes into the story",
    "moment of quiet",
    "safe at last",
    "hard-won",
];

/// Generic titles. A title must also be specific to this adventure and differ from its draft.
const STOCK_TITLES: &[&str] = &[
    "a moment of quiet",
    "hard-won ground",
    "safe at last",
    "breathing room",
    "a moment to breathe",
    "out of the fight",
    "walk together",
    "whispers in the dark",
    "hard questions",
    "the errand stalled",
    "a stand at the quay",
    "trouble at the market",
    "sworn on the road",
    "the warning lost",
    "a quiet confidence",
    "a worried whisper",
    "a trusted escort",
    "steel in the shadows",
];

/// The craft notes before the rating sentence; shared by both ratings.
macro_rules! craft_before_rating {
    () => {
        "CRAFT\n\
You rewrite the plain DRAFT of each requested slot (facts.drafts, same slot name) as an experienced Dungeon Master would say it at the table. The draft only states facts; it is not wording to keep. Write fresh sentences with your own imagery and rhythm, keeping every fact.\n\
STORY, NOT RULES: narration is pure story. No game words (hit points, rests, prone, feet, damage, rolls), no instruction to the players, no filler such as \"something is about to happen\".\n\
VOICE: narration is second person and present tense in plain strong verbs. A narrator reads it aloud, so write two short sentences rather than one long one (twenty words at most), with no semicolons, parentheses, lists, rhetorical questions or the word \"suddenly\", no purple prose and no modern idiom. Narration never quotes or repeats a spoken line and never describes a speaker's voice with the words in facts.cast.\n\
LINES: a spoken line is only the character's own first-person words, with no quotation marks, stage directions or narration about them. Lines sound spoken (contractions, a catch, a trailing thought) and every speaker sounds different: let facts.cast voice, delivery and mannerism shape word choice and rhythm. A speaker sounds the same in every line.\n\
TITLES: two to five words, specific to this adventure and built on its place, object or person (like \"The Bell That Wasn't Rung\"). Never a generic mood phrase such as \"A Moment of Quiet\", \"Safe at Last\" or \"Hard Questions\".\n\
PLACE: use two or three concrete sensory details from facts.location (sound, smell, light, weather), different ones in each scene, and never the same sentence twice. Do not start two slots with the same words.\n\
CHARACTERS: each appearance gets one consistent physical beat that fits the character's archetype, mannerism and mood together. Never contradictory, such as a brisk sergeant rubbing tired eyes or a cheerful tinker breathing hard.\n\
STAKES: the patron's opening line has a clear ask, what is at stake if the heroes refuse or fail, and one human detail, in the patron's own voice. The enemy's line is a threat that reveals a motive, in a voice that fits its kind. A creature that does not speak (facts.cast.enemy.speaks is false) gets a narrated menace beat instead of a line.\n\
PAYOFF: the contact's closing line acknowledges what the heroes did, lets the table hear that the patron's secret mattered without repeating its private wording, and gives a concrete reward or tempting lead from facts.story.thanks.\n\
CONTINUITY: keep every name, place, object and creature exactly as in FACTS and write every must_mention term verbatim. Each scene moves the story on rather than restating the last. The victory scene tells how the fight ended only in general terms. Add no new objects, people, events or injuries. "
    };
}

/// The craft notes after the rating sentence; shared by both ratings.
macro_rules! craft_after_rating {
    () => {
        "\n\
SECRET: never write a slot's forbidden terms or hint at them.\n\
LOBBY: room_narration is one or two evocative sentences of place and mood that end on a pull into the story.\n\
REWRITES: when facts.rewrite_notes is present, write only those slots and fix exactly what each note says. facts.already_written holds the other slots: do not repeat their sentences or openings.\n"
    };
}

/// Craft notes for the storyteller; they never relax its fixed rules. A restrained table keeps
/// the family-table sentence; a mature one is told mature themes are allowed. Every other
/// sentence, and the always-on limits of the storyteller's own rules, are identical.
const GUIDANCE_RESTRAINED: &str = concat!(
    craft_before_rating!(),
    "Heroic fantasy for a living-room table: tense, never gory.",
    craft_after_rating!()
);
const GUIDANCE_MATURE: &str = concat!(
    craft_before_rating!(),
    mature_themes!(),
    " Describe the violence the facts state vividly and without flinching, but never invent a wound, a death or an outcome they do not state.",
    craft_after_rating!()
);

fn guidance(rating: ContentRating) -> &'static str {
    match rating {
        ContentRating::Restrained => GUIDANCE_RESTRAINED,
        ContentRating::Mature => GUIDANCE_MATURE,
    }
}

/// What each scene is, so the model can keep the story in order; the two middle scenes are
/// alternatives, one for each way the party can answer the patron.
const SCENE_ROLES: &[(&str, &str)] = &[
    (
        "room",
        "the lobby on the TV while players join; it ends by pulling into the story",
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
        "describe exactly the listed number of creatures and their weapons before the fight begins",
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Group {
    Scenes,
    Voices,
}

impl Group {
    fn label(self) -> &'static str {
        match self {
            Self::Scenes => "scenes",
            Self::Voices => "voices",
        }
    }
}

/// Closed-vocabulary label for a storyteller failure; safe for the server log.
fn error_label(error: &StorytellerError) -> &'static str {
    match error {
        StorytellerError::Off => "off",
        StorytellerError::BudgetExhausted => "budget_exhausted",
        StorytellerError::InvalidRequest(_) => "invalid_request",
        StorytellerError::DeadlineExceeded => "deadline",
        StorytellerError::Provider(_) => "provider_error",
        StorytellerError::Rejected { .. } => "rejected",
    }
}

/// One line per slot group, so a session that fell back says which group and why.
fn log_group(
    label: &str,
    elapsed: Duration,
    slot_count: usize,
    refused: &BTreeMap<String, String>,
) {
    let refused_slots = if refused.is_empty() {
        "none".to_owned()
    } else {
        refused.keys().cloned().collect::<Vec<_>>().join(",")
    };
    let reasons = if refused.is_empty() {
        "none".to_owned()
    } else {
        refused
            .iter()
            .map(|(slot, reason)| format!("{slot}:{reason}"))
            .collect::<Vec<_>>()
            .join(",")
    };
    println!(
        "STORY group={label} ms={} slots={slot_count} refused={refused_slots} reasons={reasons}",
        elapsed.as_millis(),
    );
}

fn log_group_outcomes(
    groups: &[(String, Group, BTreeSet<String>)],
    elapsed: &[Duration],
    refused: &BTreeMap<String, String>,
) {
    for ((label, _, names), elapsed) in groups.iter().zip(elapsed) {
        let group_refused = names
            .iter()
            .filter_map(|name| {
                refused
                    .get(name)
                    .map(|reason| (name.clone(), reason.clone()))
            })
            .collect();
        log_group(label, *elapsed, names.len(), &group_refused);
    }
}

/// The planned slots split into request groups: generation order (opening
/// scenes first), never mixing scenes with voices because only a voices
/// request may be told the secret.
fn slot_groups(planned: &[Planned<'_>]) -> Vec<(String, Group, BTreeSet<String>)> {
    let mut groups: Vec<(String, Group, BTreeSet<String>)> = Vec::new();
    for group in [Group::Scenes, Group::Voices] {
        let names: Vec<&str> = planned
            .iter()
            // Offer labels are deterministic authored actions with generator facts substituted;
            // they must not be rewritten into chapter titles.
            .filter(|slot| slot.group == group && slot.sheet.slot != "beat_label")
            .map(|slot| slot.name.as_str())
            .collect();
        for (index, chunk) in names.chunks(GROUP_SLOTS).enumerate() {
            groups.push((
                format!("{}_{index}", group.label()),
                group,
                chunk.iter().map(|name| (*name).to_owned()).collect(),
            ));
        }
    }
    // Opening text first: order groups by the earliest slot they contain.
    let position = |name: &str| planned.iter().position(|slot| slot.name == name);
    groups.sort_by_key(|(_, _, names)| names.iter().filter_map(|name| position(name)).min());
    groups
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
    let scene_slots = SLOTS_PER_SCENE * scenes.len();
    // Register the appended plain-data script slots in generator order. The
    // expression policy and accepted-text replay are shared with existing slots.
    let mut script_slots = Vec::new();
    if let Some(script) = &baseline.pack.beat_script {
        // A storyteller slot name is lowercase letters, digits and underscores in 32 bytes, so
        // the script's kebab-case label and scene ids are numbered in their table order.
        for number in 0..script.labels.len() {
            script_slots.push((format!("beat_label_{number}"), "beat_label", Group::Voices));
        }
        for number in 0..script.scenes.len() {
            for (slot, group) in [
                ("title", Group::Scenes),
                ("narration", Group::Scenes),
                ("line", Group::Voices),
            ] {
                script_slots.push((format!("beat_scene_{number}_{slot}"), slot, group));
            }
        }
    }
    if sheets.len() != LEADING_SLOTS + scene_slots + script_slots.len() {
        return Err(Declined::Layout);
    }
    let mut planned = Vec::new();
    for (sheet_index, sheet) in sheets.iter().enumerate() {
        if sheet.baseline.is_empty() {
            continue;
        }
        let (name, group) = match sheet_index.checked_sub(LEADING_SLOTS) {
            None => (sheet.slot.clone(), Group::Voices),
            Some(offset) if offset >= scene_slots => {
                let (name, semantic, group) = script_slots
                    .get(offset - scene_slots)
                    .ok_or(Declined::Layout)?;
                if sheet.slot != *semantic || sheet.beat != "scene-flow" {
                    return Err(Declined::Layout);
                }
                (name.clone(), *group)
            }
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

/// The secret's own phrases: the pack's secret terms without the destination and the contact's
/// names, which the closing scene is allowed to say.
fn content_terms(baseline: &GeneratedAdventure) -> Vec<String> {
    let fact = |key: &str| baseline.facts.get(key).cloned().unwrap_or_default();
    let surname = fact("contact.full")
        .split_whitespace()
        .next_back()
        .unwrap_or_default()
        .to_owned();
    let names = [fact("contact.first"), surname];
    baseline
        .pack
        .secret_terms
        .iter()
        .filter(|term| !names.iter().any(|name| name.eq_ignore_ascii_case(term)))
        .filter(|term| !term.eq_ignore_ascii_case(&fact("dest.title")))
        .cloned()
        .collect()
}

fn slot_spec(planned: &Planned<'_>, baseline: &GeneratedAdventure) -> Slot {
    let sheet = planned.sheet;
    let secret_terms = &baseline.pack.secret_terms;
    let mut mention = sheet.must_mention.clone();
    if sheet.slot == "clue" {
        mention.extend(secret_terms.first().cloned());
    }
    let draft = sheet.baseline.to_lowercase();
    // The clue may carry the secret. The closing scene may name the destination and the
    // contact but never repeats the secret's private wording. Elsewhere, only what the pattern
    // text for that very slot already says in public is allowed.
    let protected: Vec<String> = if sheet.slot == "clue" {
        Vec::new()
    } else if sheet.beat == "resolution" {
        content_terms(baseline)
    } else {
        secret_terms.clone()
    };
    let forbidden = protected
        .into_iter()
        .filter(|term| !draft.contains(&term.to_lowercase()));
    Slot::new(&planned.name, sheet.bounds)
        .mentioning(bounded_terms(mention))
        .forbidding(bounded_terms(forbidden))
}

/// The DM's fact sheet for one request. The secret is told only to a request that writes the
/// private clue or the closing lines. `rewrites` adds the notes and context of a second ask.
fn group_facts(
    baseline: &GeneratedAdventure,
    planned: &[Planned<'_>],
    with_secret: bool,
    rewrites: Option<&Rewrites>,
) -> Value {
    let fact = |key: &str| baseline.facts.get(key).cloned().unwrap_or_default();
    let encounter_fact = |key: &str| {
        baseline
            .fact_sheet
            .iter()
            .find(|sheet| sheet.beat == "confrontation" && sheet.slot == "narration")
            .and_then(|sheet| sheet.facts.get(key))
            .cloned()
            .unwrap_or_default()
    };
    let person = |prefix: &str| {
        let age = fact(&format!("{prefix}.age"));
        json!({
            "name": fact(&format!("{prefix}.first")),
            "full_name": fact(&format!("{prefix}.full")),
            "who": fact(&format!("{prefix}.desc")),
            "voice": fact(&format!("{prefix}.voice")),
            "delivery": fact(&format!("{prefix}.delivery")),
            "mannerism": fact(&format!("{prefix}.mannerism")),
            "pronoun": fact(&format!("{prefix}.sub")),
            "age_group": age_group(&age),
            "based_on_real_person": false,
            "sexual_content_allowed": sexual_content_allowed(&age, false),
        })
    };
    let speaks = planned
        .iter()
        .any(|slot| slot.name == "combat_line" && !slot.sheet.baseline.is_empty());
    let drafts: BTreeMap<&str, String> = planned
        .iter()
        .filter(|slot| {
            with_secret
                || (slot.sheet.slot != "clue"
                    && !(slot.sheet.beat == "resolution" && slot.sheet.slot == "line"))
        })
        .map(|slot| {
            (
                slot.name.as_str(),
                slot.sheet
                    .baseline
                    .replace(ROOM_INVITATION, "")
                    .trim()
                    .to_owned(),
            )
        })
        .collect();
    let mut facts = json!({
        "tone": tone_of(baseline.rating),
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
                "encounter_count": encounter_fact("encounter.count"),
                "creatures": encounter_fact("encounter.creatures"),
                "weapons": encounter_fact("encounter.weapons"),
                "pattern_fallback": encounter_fact("encounter.fallback"),
                "speaks": speaks,
                "voice": fact("ant.voice"),
                "delivery": fact("ant.delivery"),
                "age_group": age_group(&fact("ant.age")),
                "based_on_real_person": false,
                "sexual_content_allowed": sexual_content_allowed(&fact("ant.age"), false),
            },
        },
        "forbidden": forbidden_words(baseline.rating),
        "drafts": drafts,
    });
    if with_secret {
        facts["secret"] = json!({
            "told_privately_to_one_hero": fact("secret.fact"),
            "confirmed_at_the_end": fact("secret.confirm"),
        });
    }
    if let Some(rewrites) = rewrites {
        // The secret-bearing slots are context only for a request that is told the secret.
        let secret_slot = |name: &str| {
            planned.iter().any(|slot| {
                slot.name == name
                    && (slot.sheet.slot == "clue"
                        || (slot.sheet.beat == "resolution" && slot.sheet.slot == "line"))
            })
        };
        let written: BTreeMap<&String, &String> = rewrites
            .written
            .iter()
            .filter(|(name, _)| with_secret || !secret_slot(name))
            .collect();
        facts["rewrite_notes"] = json!(rewrites.notes);
        facts["previous_attempt"] = json!(rewrites.previous);
        facts["already_written"] = json!(written);
    }
    facts
}

/// The second ask: what to fix in which slot, the slot's earlier text, and the text of the
/// other slots to avoid repeating.
struct Rewrites {
    notes: BTreeMap<String, &'static str>,
    previous: BTreeMap<String, String>,
    written: BTreeMap<String, String>,
}

/// What to tell the model about a problem, by the name `find_problems` reports it under.
fn rewrite_note(problem: &str) -> &'static str {
    match problem {
        "digits" => "contains digits; write numbers in words or leave them out",
        "markup" => "contains markup or symbols; plain sentences only",
        "quotation_marks" => {
            "contains quotation marks; narration never quotes a line, and a line has no quotation marks"
        }
        "punctuation" => {
            "uses a semicolon, parenthesis or question mark; use short plain sentences"
        }
        "new_name" => "names a person or place that is not in the facts; use only the facts' names",
        "title" => {
            "the title must be two to five words, specific to this adventure's place, object or person, not a generic mood phrase and not the draft's title"
        }
        "stock_phrase" => "uses a stock phrase; say it in a fresh, specific way",
        "long_sentence" => "has a sentence over twenty-eight words; split it into short sentences",
        "repeated_sentence" => "repeats a sentence of another slot; write something new",
        "same_opening" => "starts with the same words as another slot; open differently",
        "repeated_title" => "repeats another title; make it its own",
        "narration_in_line" => {
            "reads as narration; a line is only the character's own first-person spoken words, with no narrating about anyone"
        }
        _ => "read badly; rewrite it",
    }
}

fn words(text: &str) -> Vec<&str> {
    text.split(|c: char| !c.is_alphanumeric() && c != '\'' && c != '-')
        .filter(|word| !word.is_empty())
        .collect()
}

fn sentences(text: &str) -> Vec<&str> {
    text.split_inclusive(['.', '!', '?'])
        .map(str::trim)
        .filter(|sentence| !sentence.is_empty())
        .collect()
}

/// Removes whole sentences already spoken or read in another scene.
pub(super) fn remove_repeated_sentences(text: &str, earlier: &[&str]) -> String {
    let repeated: BTreeSet<String> = earlier
        .iter()
        .flat_map(|text| sentences(text))
        .map(normalized_sentence)
        .collect();
    sentences(text)
        .into_iter()
        .filter(|sentence| !repeated.contains(&normalized_sentence(sentence)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalized_sentence(text: &str) -> String {
    words(text)
        .into_iter()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Section 7 validators that need more than the storyteller's own checks, by problem name: no
/// digits, markup or stage directions, no capitalised word that is neither a fact nor
/// sentence-initial, narration that reads aloud, and titles that belong to this adventure.
fn semantic_problem(
    sheet: &FactSheet,
    text: &str,
    known_words: &BTreeSet<String>,
    specific_words: &BTreeSet<String>,
) -> Option<&'static str> {
    let markup = [
        '*', '_', '#', '`', '<', '>', '[', ']', '{', '}', '\\', '|', '~',
    ];
    if text.contains(markup) {
        return Some("markup");
    }
    if text.chars().any(|c| c.is_ascii_digit())
        && !sheet.baseline.chars().any(|c| c.is_ascii_digit())
    {
        return Some("digits");
    }
    if sheet.slot == "title" {
        let lowered = text.to_lowercase();
        let title_words = words(&lowered);
        let specific = title_words
            .iter()
            .any(|word| specific_words.contains(word.trim_end_matches("'s")));
        return (!(2..=5).contains(&title_words.len())
            || STOCK_TITLES.contains(&lowered.trim_end_matches('.'))
            || lowered == sheet.baseline.to_lowercase()
            || !specific)
            .then_some("title");
    }
    if sheet.slot.ends_with("_label") {
        return None;
    }
    // Quotation marks in narration mean it is quoting a line; inside a line, stage business.
    if (sheet.slot == "narration" || sheet.slot == "line") && text.contains('"') {
        return Some("quotation_marks");
    }
    if text.contains([';', '(', ')']) || (sheet.slot == "narration" && text.contains('?')) {
        return Some("punctuation");
    }
    if sheet.slot == "line" && reads_as_narration(text, &sheet.speaker) {
        return Some("narration_in_line");
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
                return Some("new_name");
            }
            initial = false;
        }
        initial = token.ends_with(['.', '!', '?', ':']);
    }
    None
}

/// A spoken line is first-person speech. Text that starts with the speaker's own name, or that
/// runs six words or more without once saying I, me, my, we, us or our (and is no question),
/// is narration that slipped into a line.
fn reads_as_narration(text: &str, speaker: &str) -> bool {
    const FIRST_PERSON: &[&str] = &[
        "i", "i'm", "i'd", "i'll", "i've", "me", "my", "mine", "we", "we're", "we'll", "we've",
        "us", "our", "let's",
    ];
    let lowered = text.to_lowercase();
    let spoken = words(&lowered);
    if !speaker.is_empty()
        && spoken
            .first()
            .zip(speaker.split_whitespace().next())
            .is_some_and(|(first, name)| first.eq_ignore_ascii_case(name))
    {
        return true;
    }
    spoken.len() >= 6
        && !text.trim_end().ends_with('?')
        && !spoken.iter().any(|word| FIRST_PERSON.contains(word))
}

/// Single-slot lint: stock phrases and sentences too long to read aloud.
fn slot_lint(text: &str) -> Option<&'static str> {
    let lowered = text.to_lowercase();
    if STOCK_PHRASES.iter().any(|phrase| lowered.contains(phrase)) {
        return Some("stock_phrase");
    }
    sentences(text)
        .iter()
        .any(|sentence| words(sentence).len() > MAXIMUM_SENTENCE_WORDS)
        .then_some("long_sentence")
}

/// Two slots of which only one is ever played, so they may share sentences.
fn alternatives(a: &str, b: &str) -> bool {
    let pair = |x: &str, y: &str| {
        (a.starts_with(x) && b.starts_with(y)) || (a.starts_with(y) && b.starts_with(x))
    };
    pair("dialogue_seal", "dialogue_escort") || pair("victory", "defeat")
}

/// Every problem in the accepted text, by slot: the single-slot checks, then the checks across
/// slots (a sentence or an opening of three words used twice, a title used twice). The later
/// slot of a pair is the one reported.
fn find_problems(
    planned: &[Planned<'_>],
    texts: &BTreeMap<String, String>,
    known: &BTreeSet<String>,
    specific: &BTreeSet<String>,
) -> BTreeMap<String, &'static str> {
    let mut problems = BTreeMap::new();
    let mut seen: Vec<(&str, BTreeSet<String>, String)> = Vec::new();
    let mut titles: Vec<&str> = Vec::new();
    for slot in planned {
        let Some(text) = texts.get(&slot.name) else {
            continue;
        };
        if let Some(problem) =
            semantic_problem(slot.sheet, text, known, specific).or_else(|| slot_lint(text))
        {
            problems.insert(slot.name.clone(), problem);
            continue;
        }
        if slot.sheet.slot == "title" {
            if titles.iter().any(|title| title.eq_ignore_ascii_case(text)) {
                problems.insert(slot.name.clone(), "repeated_title");
            }
            titles.push(text);
            continue;
        }
        if slot.sheet.slot != "narration" && slot.sheet.slot != "line" {
            continue;
        }
        let own: BTreeSet<String> = sentences(text)
            .iter()
            .filter(|sentence| words(sentence).len() >= 4)
            .map(|sentence| sentence.to_lowercase())
            .collect();
        let opening: Vec<String> = words(text)
            .iter()
            .take(3)
            .map(|word| word.to_lowercase())
            .collect();
        let opening = if opening.len() == 3 {
            opening.join(" ")
        } else {
            String::new()
        };
        for (other, other_sentences, other_opening) in &seen {
            if alternatives(&slot.name, other) {
                continue;
            }
            if own.intersection(other_sentences).next().is_some() {
                problems.insert(slot.name.clone(), "repeated_sentence");
            } else if !opening.is_empty() && opening == *other_opening {
                problems.insert(slot.name.clone(), "same_opening");
            }
        }
        seen.push((&slot.name, own, opening));
    }
    problems
}

/// Words that tie a title to this adventure: its place, object, destination, people and the
/// words of the location's own details.
fn specific_words(baseline: &GeneratedAdventure) -> BTreeSet<String> {
    const PLAIN: &[&str] = &["the", "and", "that", "with", "from", "this", "into", "your"];
    [
        "loc.name",
        "loc.atmos_a",
        "loc.atmos_b",
        "loc.atmos_c",
        "stake.short",
        "stake.full",
        "dest.name",
        "dest.title",
        "patron.first",
        "patron.role",
        "patron.desc",
        "contact.first",
        "contact.role",
        "contact.desc",
        "ant.short",
        "ant.the",
    ]
    .iter()
    .filter_map(|key| baseline.facts.get(*key))
    .flat_map(|text| {
        words(text)
            .into_iter()
            .map(str::to_lowercase)
            .collect::<Vec<_>>()
    })
    .map(|word| word.trim_end_matches("'s").to_owned())
    .filter(|word| word.chars().count() >= 4 && !PLAIN.contains(&word.as_str()))
    .collect()
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
        if let Some(Some(text)) = self.texts.get(index)
            && text.chars().count() <= sheet.bounds
            && !text.contains(['{', '}'])
            && sheet
                .must_mention
                .iter()
                .all(|term| text.to_lowercase().contains(&term.to_lowercase()))
            && sheet
                .forbidden
                .iter()
                .all(|term| !contains_whole_word(&text.to_lowercase(), &term.to_lowercase()))
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

/// One request's accepted text, the reasons for the rest and the tokens spent.
#[derive(Default)]
struct Answer {
    accepted: BTreeMap<String, String>,
    refused: BTreeMap<String, String>,
    tokens: u64,
}

async fn ask(
    storyteller: &Storyteller,
    baseline: &GeneratedAdventure,
    planned: &[Planned<'_>],
    names: Option<&BTreeSet<String>>,
    group: Option<Group>,
    rewrites: Option<&Rewrites>,
    deadline: Duration,
) -> Result<Answer, StorytellerError> {
    let asked: Vec<&Planned<'_>> = planned
        .iter()
        .filter(|slot| group.is_none_or(|group| slot.group == group))
        .filter(|slot| names.is_none_or(|names| names.contains(&slot.name)))
        .collect();
    if asked.is_empty() {
        return Ok(Answer::default());
    }
    let slots: Vec<Slot> = asked.iter().map(|slot| slot_spec(slot, baseline)).collect();
    let with_secret = asked.iter().any(|slot| slot.group == Group::Voices);
    let facts = group_facts(baseline, planned, with_secret, rewrites);
    let answer = storyteller
        .express_adventure_partial(&facts, &slots, guidance(baseline.rating), deadline)
        .await?;
    Ok(Answer {
        accepted: answer.accepted,
        refused: answer
            .rejected
            .iter()
            .map(|(slot, failure)| (slot.clone(), failure.label().to_owned()))
            .collect(),
        tokens: answer.tokens,
    })
}

/// One slot group, with one bounded retry for a deadline or provider failure.
/// A validation refusal is not retried here: the storyteller already re-asks
/// refused slots once inside the call.
async fn ask_group(
    storyteller: &Storyteller,
    baseline: &GeneratedAdventure,
    planned: &[Planned<'_>],
    group: Group,
    names: &BTreeSet<String>,
    group_deadline: Duration,
) -> (Result<Answer, StorytellerError>, Duration) {
    let started = Instant::now();
    let mut attempt = 0_u32;
    loop {
        attempt += 1;
        match ask(
            storyteller,
            baseline,
            planned,
            Some(names),
            Some(group),
            None,
            group_deadline,
        )
        .await
        {
            Ok(answer) => {
                return (Ok(answer), started.elapsed());
            }
            Err(error) => {
                let retryable = matches!(
                    error,
                    StorytellerError::DeadlineExceeded | StorytellerError::Provider(_)
                );
                if retryable && attempt < 2 {
                    continue;
                }
                return (Err(error), started.elapsed());
            }
        }
    }
}

/// The cleaned text of an answer: plain punctuation, and a spoken line without its quotes.
fn cleaned(planned: &[Planned<'_>], answer: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    planned
        .iter()
        .filter_map(|slot| {
            let raw = answer.get(&slot.name)?;
            let text = if slot.sheet.slot == "line" {
                without_outer_quotes(raw)
            } else {
                raw.trim()
            };
            Some((slot.name.clone(), plain_punctuation(text)))
        })
        .collect()
}

/// What the whole step produced: the accepted text by slot, why the others were refused, and
/// the tokens spent.
struct Written {
    texts: BTreeMap<String, String>,
    refused: BTreeMap<String, String>,
    tokens: u64,
}

async fn collect_replenishing<T, F, MakeRequest>(
    count: usize,
    concurrency: usize,
    deadline: Instant,
    mut make_request: MakeRequest,
) -> (Vec<Option<T>>, Vec<Option<Instant>>, bool)
where
    F: std::future::Future<Output = T>,
    MakeRequest: FnMut(usize) -> F,
{
    use futures::StreamExt;

    async fn indexed<T, F>(index: usize, future: F) -> (usize, T)
    where
        F: std::future::Future<Output = T>,
    {
        (index, future.await)
    }

    let mut results: Vec<Option<T>> = (0..count).map(|_| None).collect();
    let mut started_at: Vec<Option<Instant>> = (0..count).map(|_| None).collect();
    if Instant::now() >= deadline {
        return (results, started_at, count > 0);
    }
    let mut pending = futures::stream::FuturesUnordered::new();
    let mut next = 0;
    let mut deadline_hit = false;
    while next < count && next < concurrency {
        let index = next;
        let future = make_request(index);
        started_at[index] = Some(Instant::now());
        pending.push(indexed(index, future));
        next += 1;
    }
    let deadline_at = deadline;
    let deadline = tokio::time::Instant::from_std(deadline_at);
    while !pending.is_empty() {
        match tokio::time::timeout_at(deadline, pending.next()).await {
            Ok(Some((index, result))) => {
                results[index] = Some(result);
                if next < count && Instant::now() < deadline_at {
                    let index = next;
                    let future = make_request(index);
                    started_at[index] = Some(Instant::now());
                    pending.push(indexed(index, future));
                    next += 1;
                }
            }
            Ok(None) => break,
            Err(_) => {
                deadline_hit = true;
                break;
            }
        }
    }
    (results, started_at, deadline_hit || next < count)
}

async fn write(
    storyteller: &Storyteller,
    baseline: &GeneratedAdventure,
    planned: &[Planned<'_>],
    started: Instant,
) -> Result<Written, Declined> {
    write_with_limits(
        storyteller,
        baseline,
        planned,
        started,
        DEADLINE,
        GROUP_DEADLINE,
        GROUP_CONCURRENCY,
    )
    .await
}

async fn write_with_limits(
    storyteller: &Storyteller,
    baseline: &GeneratedAdventure,
    planned: &[Planned<'_>],
    started: Instant,
    overall_budget: Duration,
    group_deadline: Duration,
    concurrency: usize,
) -> Result<Written, Declined> {
    // Small groups start in generation order; a finished group frees a request slot immediately.
    let groups = slot_groups(planned);
    let deadline = started + overall_budget;
    let (results, started_at, deadline_hit) =
        collect_replenishing(groups.len(), concurrency, deadline, |index| {
            let (_, group, names) = &groups[index];
            ask_group(
                storyteller,
                baseline,
                planned,
                *group,
                names,
                group_deadline,
            )
        })
        .await;
    let mut raw: BTreeMap<String, String> = BTreeMap::new();
    let mut refused: BTreeMap<String, String> = BTreeMap::new();
    let mut tokens = 0_u64;
    let mut failure = None;
    let now = Instant::now();
    let mut elapsed: Vec<Duration> = started_at
        .into_iter()
        .map(|started| started.map_or(Duration::ZERO, |started| now.duration_since(started)))
        .collect();
    for (((_, _, names), result), elapsed) in groups.iter().zip(results).zip(&mut elapsed) {
        match result {
            Some((Ok(answer), group_elapsed)) => {
                *elapsed = group_elapsed;
                raw.extend(answer.accepted);
                refused.extend(answer.refused);
                tokens = tokens.saturating_add(answer.tokens);
            }
            Some((Err(error), group_elapsed)) => {
                *elapsed = group_elapsed;
                for name in names {
                    refused.insert(name.clone(), error_label(&error).to_owned());
                }
                failure = Some(error);
            }
            None => {
                for name in names {
                    refused.insert(name.clone(), "deadline".to_owned());
                }
            }
        }
    }
    if raw.is_empty() {
        log_group_outcomes(&groups, &elapsed, &refused);
        if deadline_hit {
            return Err(Declined::Deadline);
        }
        return Err(failure.map_or(Declined::NothingAccepted, Declined::Storyteller));
    }
    let known = known_words(baseline);
    let specific = specific_words(baseline);
    let mut texts = cleaned(planned, &raw);
    let mut problems = find_problems(planned, &texts, &known, &specific);
    let remaining = DEADLINE.saturating_sub(started.elapsed());
    if !problems.is_empty() && remaining >= REASK_MINIMUM {
        let rewrites = Rewrites {
            notes: problems
                .iter()
                .map(|(slot, problem)| (slot.clone(), rewrite_note(problem)))
                .collect(),
            previous: problems
                .keys()
                .filter_map(|slot| texts.get(slot).map(|text| (slot.clone(), text.clone())))
                .collect(),
            written: texts
                .iter()
                .filter(|(slot, _)| !problems.contains_key(*slot))
                .map(|(slot, text)| (slot.clone(), text.clone()))
                .collect(),
        };
        let names: BTreeSet<String> = problems.keys().cloned().collect();
        // A failed second ask costs only the rewrites; the first answers stand until the
        // lint below drops whatever is still wrong.
        if let Ok(second) = ask(
            storyteller,
            baseline,
            planned,
            Some(&names),
            None,
            Some(&rewrites),
            remaining,
        )
        .await
        {
            tokens = tokens.saturating_add(second.tokens);
            texts.extend(cleaned(planned, &second.accepted));
            problems = find_problems(planned, &texts, &known, &specific);
        }
    }
    for (slot, problem) in &problems {
        texts.remove(slot);
        refused.insert(slot.clone(), format!("lint_{problem}"));
    }
    log_group_outcomes(&groups, &elapsed, &refused);
    Ok(Written {
        texts,
        refused,
        tokens,
    })
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
    let started = Instant::now();
    let written = write(storyteller, baseline, &planned, started).await?;
    let mut texts: Vec<Option<String>> = vec![None; baseline.fact_sheet.len()];
    for slot in &planned {
        if let (Some(text), Some(entry)) = (
            written.texts.get(&slot.name),
            texts.get_mut(slot.sheet_index),
        ) {
            *entry = Some(text.clone());
        }
    }
    let accepted = AcceptedText {
        texts,
        position: Cell::new(0),
        used: RefCell::new(BTreeSet::new()),
    };
    let mut rebuilt =
        generate_with_prose(tables, seed, options, forced, &accepted).map_err(Declined::Replay)?;
    super::campaign_render::remove_repeated_ending_sentences(&mut rebuilt, Some(baseline));
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
            written
                .refused
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
            tokens: written.tokens,
            elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        },
    ))
}

#[cfg(test)]
#[path = "expression_tests.rs"]
mod tests;

#[cfg(test)]
mod regression_tests {
    use super::remove_repeated_sentences;

    #[test]
    fn repeated_ending_sentence_is_removed_without_dropping_new_prose() {
        assert_eq!(
            remove_repeated_sentences(
                "The fire warms the room. Ice glitters on the old waystation walls.",
                &["Ice glitters on the old waystation walls."],
            ),
            "The fire warms the room."
        );
    }

    #[test]
    fn all_repeated_ending_sentences_leave_no_text_for_table_fallback() {
        assert!(
            remove_repeated_sentences(
                "Ice glitters on the old waystation walls.",
                &["Ice glitters on the old waystation walls."],
            )
            .is_empty()
        );
    }
}
