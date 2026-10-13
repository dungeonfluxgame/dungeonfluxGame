//! The scene's speaker answers, in character, what a player types to them.
//!
//! The reply is prose only. It is produced after the native actor has classified the typed
//! line, off the actor thread, and is delivered solely in that player's own dialogue
//! response. A typed line never executes an action or a check; the reply cannot either,
//! because the storyteller rejects game numbers that are not in the facts.
//!
//! Listener safety: the facts hold only what the asking player has already been told. The
//! scene's secret clue is included only when this member has privately received it, and the
//! facts then say so plainly so the speaker does not deflect. Otherwise the pack's explicit
//! `secret_terms` (minus anything the whole room already sees) go into `forbidden`, and the
//! storyteller refuses any reply that uses them. A pack without that list falls back to the
//! proper names found in the clue text.

use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};

use df_protocol::common as rpc;
use df_types::MemberId;
use serde_json::{Value, json};

use super::{
    adventure::dossier::{Dossier, NpcRole, mentions_term},
    approach_model::ApproachModel,
    npc_dossier::{self, Clue, Inputs, Turn, Withheld, released_terms},
    pack::{Age, PackFile, SceneKey, SpeakerProfile},
    social::{Approach, Shift, SocialLedger, classify_keywords, shift_line},
    speech_text::{coverage, sentences},
    startup_configuration::content_rating,
    storyteller::{
        CallProfile, Expression, Slot, Storyteller, StorytellerError, age_group,
        contains_whole_word, sexual_content_allowed,
    },
    table_log::TableLog,
};

/// Longest in-character reply, in characters.
pub(super) const REPLY_CHARACTERS: usize = 220;
/// Only this much of the player's words is shown to the model.
const MAXIMUM_PLAYER_CHARACTERS: usize = 500;
const REPLIES_PER_WINDOW: usize = 6;
const WINDOW: Duration = Duration::from_secs(60);
/// How much of an offered line the reply must say for the line to count as said.
const USED_COVERAGE: f64 = 0.45;

const STYLE: &str = "A spoken line in character, in the first person, with no quotation marks, \
shaped by facts.npc (role, voice). Short plain words, one thought at a time, never a semicolon \
or a colon. It deflects only what facts.forbidden or the scene hides; when \
the hero was told the secret, it speaks of it plainly and quietly instead. An enemy taunts, \
threatens or bargains and never helps. It never promises mechanical results. There are no menus \
in this room, only people talking: never name choice labels and never tell the player what to \
click or ask. Plain and brief.";

/// What the pack says about the story that a reply needs: the private clue, the phrases that
/// give it away and who each speaking character is.
pub(super) struct StoryFacts {
    clue: String,
    secret_terms: Vec<String>,
    /// Names the authored scenes already speak to the whole table; never withheld.
    public_names: Vec<String>,
    speakers: Vec<SpeakerProfile>,
    /// Each scene's spoken line, with the scene's place in the story and its name.
    scene_lines: Vec<(usize, String, String)>,
}

impl StoryFacts {
    pub(super) fn from_pack(pack: &PackFile) -> Self {
        Self {
            clue: pack.courier_clue.clone(),
            secret_terms: pack.secret_terms.clone(),
            public_names: spoken_names(pack),
            speakers: pack.speakers.clone(),
            scene_lines: pack
                .scenes
                .iter()
                .filter_map(|scene| {
                    let order = SceneKey::ALL.iter().position(|key| *key == scene.key)?;
                    Some((order, format!("{:?}", scene.key), scene.dialogue.clone()))
                })
                .collect(),
        }
    }

    /// The statements the story has speakers make only in scenes after `scene`: the speaker
    /// cannot know them yet, so a reply must not offer them.
    fn later_lines(&self, scene: &str) -> Vec<String> {
        let Some((now, _, _)) = self.scene_lines.iter().find(|(_, name, _)| name == scene) else {
            return Vec::new();
        };
        let statements = |keep: &dyn Fn(usize) -> bool| -> Vec<String> {
            self.scene_lines
                .iter()
                .filter(|(order, _, _)| keep(*order))
                .flat_map(|(_, _, dialogue)| sentences(dialogue))
                .collect()
        };
        let known = statements(&|order| order <= *now);
        statements(&|order| order > *now)
            .into_iter()
            .filter(|line| !known.contains(line))
            .collect()
    }
}

/// The age and likeness facts that bound what a speaker may be written into, in either rating.
/// A speaker's age comes from the pack's profile and stays unstated when the pack has none;
/// `based_on_real_person` is false for every character today, because no character is cast
/// from a real person's likeness.
fn character_limits(profile: Option<&SpeakerProfile>) -> Value {
    let age = match profile.and_then(|profile| profile.age) {
        Some(Age::Young) => "young",
        Some(Age::Adult) => "adult",
        Some(Age::Elder) => "elder",
        None => "",
    };
    json!({
        "age_group": age_group(age),
        "based_on_real_person": false,
        "sexual_content_allowed": sexual_content_allowed(age, false),
    })
}

/// Adds [`character_limits`] to the `npc` object of the reply facts.
fn add_character_limits(facts: &mut Value, profile: Option<&SpeakerProfile>) {
    if let (Some(npc), Value::Object(limits)) =
        (facts["npc"].as_object_mut(), character_limits(profile))
    {
        npc.extend(limits);
    }
}

/// Everything one reply needs, captured by the actor and owned so the model call can run
/// elsewhere.
pub(super) struct NpcReplyContext {
    member: MemberId,
    pub(super) facts: Value,
    pub(super) speaker: String,
    hero_name: String,
    hero_index: u32,
    pub(super) typed_words: String,
    public_scene: String,
    /// Names present in the scene (heroes, cast, places): the only names a cleaned-up line may
    /// be corrected to.
    known_names: Vec<String>,
    /// Set when the addressed NPC has a backstory: the reply then draws on it and moves the
    /// NPC's attitude toward this hero.
    pub(super) turn: Option<Box<Turn>>,
}

/// How one exchange with an NPC that has a backstory went.
pub(super) struct Exchange {
    pub(super) text: String,
    /// What the hero's words did, as the model or the keyword reader read them.
    pub(super) approach: Approach,
    /// False when the dossier's prepared answer replied instead of the text model.
    pub(super) from_model: bool,
    /// Why the model reply never spoke, when it did not. Closed vocabulary, never reply text.
    pub(super) fallback: Option<FallbackReason>,
    pub(super) shift: Option<Shift>,
    /// The reply spoke of what was whispered to this hero alone, so only this hero was shown it.
    pub(super) private: bool,
}

/// Why a model reply never spoke. Closed vocabulary for the one server line each fallback
/// prints; it never carries reply text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FallbackReason {
    /// The storyteller is off: no call was made.
    ModelOff,
    /// The member talked past the reply bounds (six a minute): no call was made.
    RateLimited,
    /// The per-run token budget could not cover the call.
    BudgetExhausted,
    /// The model call failed or timed out.
    CallFailed,
    /// The storyteller refused the request itself: the label names what was wrong, today
    /// "style" when the guidance outgrew its bound, "slot ..." for a malformed slot.
    RequestInvalid(&'static str),
    /// The model answered, but its reply broke a rule on both attempts.
    ReplyRejected,
    /// The model answered, but the reply was empty.
    EmptyReply,
}

impl FallbackReason {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::ModelOff => "model_off",
            Self::RateLimited => "rate_limited",
            Self::BudgetExhausted => "budget_exhausted",
            Self::CallFailed => "call_failed",
            Self::RequestInvalid(_) => "request_invalid",
            Self::ReplyRejected => "reply_rejected",
            Self::EmptyReply => "empty_reply",
        }
    }
}

/// Who a typed line is for: the one on-stage NPC the words name, otherwise whoever is
/// speaking, otherwise the only NPC on stage.
fn addressee<'a>(
    present: &[(&'a rpc::SceneActor, &'a Dossier)],
    words: &str,
) -> Option<(&'a rpc::SceneActor, &'a Dossier)> {
    let named: Vec<(&rpc::SceneActor, &Dossier)> = present
        .iter()
        .filter(|(actor, dossier)| {
            mentions_term(words, &dossier.name) || mentions_term(words, &actor.name)
        })
        .copied()
        .collect();
    if let [only] = named.as_slice() {
        return Some(*only);
    }
    present
        .iter()
        .find(|(actor, _)| actor.speaking)
        .or_else(|| (present.len() == 1).then(|| &present[0]))
        .copied()
}

impl NpcReplyContext {
    /// The context for a line typed to whoever on stage it addresses: an NPC with a dossier
    /// when the adventure has them, otherwise (and for packs without dossiers) the scene's
    /// single speaker as before. `scene` names the current scene for the attitude bounds.
    pub(super) fn for_cast(
        view: &rpc::ViewMessage,
        member: MemberId,
        typed: &str,
        story: &StoryFacts,
        social: &Arc<SocialLedger>,
        scene: &str,
    ) -> Option<Self> {
        if social.is_active()
            && let Some(context) = Self::with_dossier(view, member, typed, story, social, scene)
        {
            return Some(context);
        }
        Self::from_view(view, member, typed, story)
    }

    fn with_dossier(
        view: &rpc::ViewMessage,
        member: MemberId,
        typed: &str,
        story: &StoryFacts,
        social: &Arc<SocialLedger>,
        scene_key: &str,
    ) -> Option<Self> {
        let Some(rpc::view_message::Audience::Player(player)) = &view.audience else {
            return None;
        };
        let journey = player.journey.as_ref()?;
        let hero = journey.own_character.as_ref()?;
        let words = sanitize_player_words(typed);
        if words.is_empty() || hero.name.is_empty() {
            return None;
        }
        let scene = player.scene.as_ref()?;
        let present: Vec<(&rpc::SceneActor, &Dossier)> = scene
            .cast
            .iter()
            .filter_map(|actor| {
                let role = rpc::SceneRole::try_from(actor.role).ok()?;
                Some((actor, social.dossier_of(role, &actor.id)?))
            })
            .collect();
        let (actor, dossier) = addressee(&present, &words)?;
        let choices: Vec<&str> = player
            .offers
            .iter()
            .filter(|offer| offer.action_kind != rpc::GameplayActionKind::CreateCharacter as i32)
            .map(|offer| offer.label.as_str())
            .collect();
        let on_stage: Vec<&str> = scene.cast.iter().map(|actor| actor.name.as_str()).collect();
        let public = format!(
            "{} {} {} {} {} {}",
            scene.title,
            player.narration,
            journey.dialogue,
            hero.name,
            choices.join(" "),
            on_stage.join(" "),
        );
        // Only the speaker who whispered the clue speaks of it, and only to the hero who was
        // told it: that hero is given the clue's secret terms, no other hero is.
        let gave_clue = dossier.role == NpcRole::Patron
            && !story.clue.is_empty()
            && mentions_term(&story.clue, &dossier.name);
        let told = gave_clue && !player.private_clue.is_empty();
        let released = if told {
            // Names the scenes say aloud are not the clue's to give: the table already hears them.
            let clue_terms: Vec<String> = story
                .secret_terms
                .iter()
                .filter(|term| {
                    !story
                        .public_names
                        .iter()
                        .any(|name| name.eq_ignore_ascii_case(term))
                })
                .cloned()
                .collect();
            released_terms(dossier, &clue_terms, &player.private_clue)
        } else {
            Vec::new()
        };
        let clue = if told {
            Clue::Told(player.private_clue.as_str())
        } else if gave_clue {
            Clue::Kept
        } else {
            Clue::Absent
        };
        let withheld = Withheld::new(
            dossier,
            unheard_secret_terms(story, &public),
            released,
            content_rating(),
        );
        let standing = social.standing(member, dossier);
        let recent = social.recent(member, &dossier.npc_id);
        let scripted = if actor.speaking {
            journey.dialogue.as_str()
        } else {
            ""
        };
        let conversation = social.conversation(member, dossier, scene_key);
        let later_lines = story.later_lines(scene_key);
        let present: Vec<(String, String)> = scene
            .cast
            .iter()
            .filter_map(|actor| {
                let role = rpc::SceneRole::try_from(actor.role).ok()?;
                Some((actor.name.clone(), format!("{role:?}")))
            })
            .collect();
        let inputs = Inputs {
            dossier,
            name: &actor.name,
            hero: &hero.name,
            standing,
            recent: &recent,
            scene_title: &scene.title,
            scripted_line: scripted,
            words: &words,
            withheld: &withheld,
            conversation: &conversation,
            later_lines: &later_lines,
            clue,
        };
        let (mut facts, offered, said_lines) =
            npc_dossier::facts_with_scene(&inputs, &player.narration, &present);
        let answer_offered = ["answer_from", "you_told_this_hero_in_private"]
            .iter()
            .any(|key| match &facts[*key] {
                Value::Array(lines) => !lines.is_empty(),
                Value::String(line) => !line.is_empty(),
                _ => false,
            });
        // Everything the speaker has already said to this hero: what a new reply must not
        // repeat. The prompt carries only the most recent of the dossier lines; the checks
        // use the whole scene, so a line from ten exchanges back is not said again.
        let mut said: Vec<String> = recent
            .iter()
            .filter(|(from_hero, _)| !from_hero)
            .map(|(_, text)| text.clone())
            .chain(said_lines)
            .collect();
        said.sort();
        said.dedup();
        add_character_limits(
            &mut facts,
            story
                .speakers
                .iter()
                .find(|speaker| speaker.name == actor.name),
        );
        let hero_index = journey
            .party
            .iter()
            .position(|party_member| party_member.name == hero.name)
            .and_then(|position| u32::try_from(position).ok())
            .unwrap_or(0);
        Some(Self {
            member,
            facts,
            speaker: actor.name.clone(),
            hero_name: hero.name.clone(),
            hero_index,
            typed_words: words,
            public_scene: player.narration.clone(),
            known_names: scene_names(journey, player.scene.as_ref()),
            turn: Some(Box::new(Turn {
                ledger: social.clone(),
                dossier: dossier.clone(),
                scene: scene_key.to_owned(),
                standing,
                withheld,
                offered,
                said,
                conversation,
                answer_offered,
            })),
        })
    }

    /// Builds the context from the asking player's own view. `None` when nobody is speaking
    /// to the party, the hero is not ready, or the typed text is empty.
    pub(super) fn from_view(
        view: &rpc::ViewMessage,
        member: MemberId,
        typed: &str,
        story: &StoryFacts,
    ) -> Option<Self> {
        let Some(rpc::view_message::Audience::Player(player)) = &view.audience else {
            return None;
        };
        let journey = player.journey.as_ref()?;
        let hero = journey.own_character.as_ref()?;
        if journey.speaker.is_empty() || journey.dialogue.is_empty() {
            return None;
        }
        let words = sanitize_player_words(typed);
        if words.is_empty() || hero.name.is_empty() {
            return None;
        }
        let title = player.scene.as_ref().map(|scene| scene.title.as_str());
        let choices: Vec<&str> = player
            .offers
            .iter()
            .filter(|offer| offer.action_kind != rpc::GameplayActionKind::CreateCharacter as i32)
            .map(|offer| offer.label.as_str())
            .collect();
        let profile = story
            .speakers
            .iter()
            .find(|speaker| speaker.name == journey.speaker);
        let mut facts = json!({
            "npc": {
                "name": journey.speaker,
                "role": profile.map_or("the speaker the scene narration describes", |speaker| {
                    speaker.role.as_str()
                }),
                "voice": profile.map_or("", |speaker| speaker.voice.as_str()),
            },
            "scene": {
                "title": title.unwrap_or_default(),
                "narration": player.narration,
            },
            "npc_current_scripted_line": journey.dialogue,
            "player": {"hero_name": hero.name, "exact_words": words},
            // Menu labels stay out of the facts: a reply that names them reads the phone's
            // menu aloud instead of answering in character.
            "offered_choices": Vec::<&str>::new(),
        });
        add_character_limits(&mut facts, profile);
        // Typed conversation is public table talk, so the secret's terms stay forbidden for
        // every asker; the room hears the reply.
        let public = format!(
            "{} {} {} {} {}",
            title.unwrap_or_default(),
            player.narration,
            journey.dialogue,
            hero.name,
            choices.join(" "),
        );
        facts["forbidden"] = json!(unheard_secret_terms(story, &public));
        if player.private_clue.is_empty() {
            facts["listener_knowledge"] = json!(
                "This hero has NOT been told the secret. Never reveal it or hint at the \
                 forbidden terms; deflect questions about them in character."
            );
        } else {
            facts["secret_clue_this_player_was_privately_told"] = json!(player.private_clue);
            facts["listener_knowledge"] = json!(
                "This hero knows your secret; others are listening, so allude to it without \
                 stating it. Never use the forbidden terms."
            );
        }
        let hero_index = journey
            .party
            .iter()
            .position(|party_member| party_member.name == hero.name)
            .and_then(|position| u32::try_from(position).ok())
            .unwrap_or(0);
        Some(Self {
            member,
            facts,
            speaker: journey.speaker.clone(),
            hero_name: hero.name.clone(),
            hero_index,
            typed_words: words,
            public_scene: player.narration.clone(),
            known_names: scene_names(journey, player.scene.as_ref()),
            turn: None,
        })
    }

    /// Publishes the hero's line as public table talk, before any answer. `shown` is the display
    /// version (see `line_cleanup`); the raw words stay what the reply reasons about. A hero who
    /// was told a clue in private may type its terms, but the table does not read them.
    pub(super) fn publish_typed_line(&self, log: &TableLog, shown: &str) {
        let spoken = match &self.turn {
            Some(turn) => turn.withheld.hide_released(shown),
            None => shown.to_owned(),
        };
        log.push(
            &self.hero_name,
            rpc::TableRole::Hero,
            self.hero_index,
            rpc::TableKind::Speech,
            &spoken,
        );
    }
}

/// The names a scene shows: its heroes, cast, speaker and the capitalised words of its title.
fn scene_names(journey: &rpc::JourneyView, scene: Option<&rpc::GameplayScene>) -> Vec<String> {
    let mut names: Vec<String> = journey
        .party
        .iter()
        .map(|member| member.name.clone())
        .chain(std::iter::once(journey.speaker.clone()))
        .collect();
    if let Some(scene) = scene {
        names.extend(scene.cast.iter().map(|actor| actor.name.clone()));
        names.extend(
            scene
                .title
                .split_whitespace()
                .filter(|word| word.chars().next().is_some_and(char::is_uppercase))
                // "The Broken Seal" names a place; "The" alone must not capitalise every "the".
                .filter(|word| {
                    !["the", "a", "an", "of", "and", "at", "in", "on", "to", "for"]
                        .contains(&word.to_lowercase().as_str())
                })
                .map(str::to_owned),
        );
    }
    names.retain(|name| !name.trim().is_empty());
    names.sort();
    names.dedup();
    names
}

/// Control characters become spaces and only the first part of the text is kept.
fn sanitize_player_words(typed: &str) -> String {
    typed
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(MAXIMUM_PLAYER_CHARACTERS)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// The pack's explicit secret terms the whole room has not already seen; a pack that lists none
/// falls back to the proper names in its clue text. The speakers' names and the destination are
/// not secret even when the pack lists them: the authored scenes say them aloud, and an NPC
/// talking about her errand must be able to name them.
fn unheard_secret_terms(story: &StoryFacts, public: &str) -> Vec<String> {
    if story.secret_terms.is_empty() {
        return secret_terms(&story.clue, public);
    }
    let public = public.to_lowercase();
    story
        .secret_terms
        .iter()
        .filter(|term| !contains_whole_word(&public, &term.to_lowercase()))
        .filter(|term| {
            !story
                .public_names
                .iter()
                .any(|name| name.eq_ignore_ascii_case(term))
        })
        .cloned()
        .collect()
}

/// The names the authored scenes speak aloud: every speaker and the destination without its
/// article ("the Net Loft" is spoken as "Net Loft").
pub(super) fn spoken_names(pack: &PackFile) -> Vec<String> {
    let mut names: Vec<String> = pack
        .speakers
        .iter()
        .map(|speaker| speaker.name.clone())
        .collect();
    if let Some(label) = &pack.destination_label {
        let article = label
            .get(..4)
            .is_some_and(|start| start.eq_ignore_ascii_case("the "));
        names.push(if article {
            label[4..].to_owned()
        } else {
            label.clone()
        });
    }
    names.retain(|name| !name.trim().is_empty());
    names
}

/// Proper names in the clue (runs of capitalized words after the first word of a sentence), minus anything the whole room already sees in `public`.
fn secret_terms(clue: &str, public: &str) -> Vec<String> {
    let public = public.to_lowercase();
    let mut terms: Vec<String> = Vec::new();
    let mut run: Vec<&str> = Vec::new();
    let mut sentence_start = true;
    /// A run of names becomes the whole phrase and its last word; a lone name is itself.
    fn flush(run: &mut Vec<&str>, terms: &mut Vec<String>) {
        if run.len() > 1 {
            terms.push(run.join(" "));
        }
        if let Some(last) = run.last() {
            terms.push((*last).to_owned());
        }
        run.clear();
    }
    for token in clue.split_whitespace() {
        let word = token.trim_matches(|character: char| !character.is_alphanumeric());
        let capitalized = word.chars().next().is_some_and(char::is_uppercase);
        if capitalized && !sentence_start && word.chars().count() >= 3 {
            run.push(word);
        } else {
            flush(&mut run, &mut terms);
        }
        sentence_start = token.ends_with(['.', '!', '?', ':']);
        if sentence_start {
            flush(&mut run, &mut terms);
        }
    }
    flush(&mut run, &mut terms);
    let mut kept: Vec<String> = Vec::new();
    for term in terms {
        if !contains_whole_word(&public, &term.to_lowercase())
            && !kept
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(&term))
        {
            kept.push(term);
        }
    }
    kept
}

/// Bounds replies per member: one in flight and a few per minute.
#[derive(Default)]
struct ReplyLimiter {
    members: Mutex<BTreeMap<MemberId, MemberWindow>>,
}

#[derive(Default)]
struct MemberWindow {
    in_flight: bool,
    starts: VecDeque<Instant>,
}

/// Held while a reply is being produced; dropping it frees the member's single slot.
struct Admission {
    limiter: Arc<ReplyLimiter>,
    member: MemberId,
}

impl Drop for Admission {
    fn drop(&mut self) {
        let mut members = self
            .limiter
            .members
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(window) = members.get_mut(&self.member) {
            window.in_flight = false;
        }
    }
}

impl ReplyLimiter {
    fn admit(self: &Arc<Self>, member: MemberId, now: Instant) -> Option<Admission> {
        let mut members = self.members.lock().unwrap_or_else(PoisonError::into_inner);
        members.retain(|_, window| {
            while window
                .starts
                .front()
                .is_some_and(|start| now.duration_since(*start) >= WINDOW)
            {
                window.starts.pop_front();
            }
            window.in_flight || !window.starts.is_empty()
        });
        let window = members.entry(member).or_default();
        if window.in_flight || window.starts.len() >= REPLIES_PER_WINDOW {
            return None;
        }
        window.in_flight = true;
        window.starts.push_back(now);
        Some(Admission {
            limiter: self.clone(),
            member,
        })
    }
}

/// The server's NPC voice: asks the storyteller, within the per-member bounds.
pub(super) struct NpcVoice {
    storyteller: Arc<Storyteller>,
    limiter: Arc<ReplyLimiter>,
    approach_model: ApproachModel,
}

impl NpcVoice {
    pub(super) fn new(storyteller: Arc<Storyteller>) -> Self {
        Self {
            storyteller,
            limiter: Arc::default(),
            approach_model: ApproachModel::default(),
        }
    }

    pub(super) fn with_approach_model(mut self, approach_model: ApproachModel) -> Self {
        self.approach_model = approach_model;
        self
    }

    /// A voice that never answers.
    #[cfg(test)]
    pub(super) fn silent() -> Self {
        Self::new(Arc::new(Storyteller::disabled()))
    }

    /// The reply text, or `None` when the storyteller is off, the member is over its bounds,
    /// or the model failed, timed out or broke a rule. `None` means "answer as before".
    /// [`Self::reply`], and the reply also becomes public table talk: the asker gets it in the
    /// dialogue response and the whole room sees it in the table log. A reply that never came
    /// adds nothing, so only the player's own line stands.
    pub(super) async fn reply_in_public(
        &self,
        context: NpcReplyContext,
        log: &TableLog,
    ) -> Option<String> {
        let speaker = context.speaker.clone();
        let reply = self.reply(context).await?;
        log.push(
            &speaker,
            rpc::TableRole::Npc,
            0,
            rpc::TableKind::Speech,
            &reply,
        );
        Some(reply)
    }

    /// Answers a typed line for the table. An NPC without a backstory answers as before (`None`
    /// when no reply came). One with a backstory always answers in character: the text model's
    /// reply when it is on and well behaved, the dossier's prepared answer otherwise. The reply
    /// is public table talk; the exchange then moves the NPC's attitude toward the hero, which is
    /// remembered, saved and, when it changes, announced as a short public line.
    pub(super) async fn converse(
        &self,
        context: NpcReplyContext,
        log: &TableLog,
    ) -> Option<String> {
        if context.turn.is_none() {
            return self.reply_in_public(context, log).await;
        }
        let exchange = self.converse_in_character(context, log).await?;
        let operation = df_observe::OperationContext {
            trace_parent: String::new(),
            build: crate::BUILD_ID.to_owned(),
        };
        let mut span = df_observe::begin(&operation, "gameplay.social.exchange");
        span.event(
            "social.exchange",
            &[
                ("social.approach", exchange.approach.label().to_owned()),
                (
                    "social.source",
                    if exchange.from_model {
                        "model"
                    } else {
                        "prepared"
                    }
                    .to_owned(),
                ),
                ("social.shift", format!("{:?}", exchange.shift)),
                ("social.private", exchange.private.to_string()),
            ],
        );
        // One line per fallback, closed vocabulary, never reply text: today nothing records
        // why a prepared line spoke instead of the model.
        if let Some(reason) = exchange.fallback {
            span.event(
                "social.fallback",
                &[("social.reason", reason.label().to_owned())],
            );
        }
        span.finish_unmeasured("replied");
        Some(exchange.text)
    }

    /// [`Self::converse`] for an NPC with a backstory, reporting how the exchange went.
    pub(super) async fn converse_in_character(
        &self,
        mut context: NpcReplyContext,
        log: &TableLog,
    ) -> Option<Exchange> {
        let approach = self
            .approach_model
            .classify(
                &context.typed_words,
                &context.speaker,
                &context.public_scene,
            )
            .await
            .unwrap_or_else(|| classify_keywords(&context.typed_words));
        context.facts["player"]["approach"] = json!(approach.label());
        let threat_shift = if approach == Approach::Intimidation {
            let turn = context.turn.as_mut()?;
            let shift = turn
                .ledger
                .threaten(context.member, &turn.dossier, &turn.scene);
            turn.standing = turn.ledger.standing(context.member, &turn.dossier);
            shift
        } else {
            None
        };
        if let Some(shift @ Shift::Level { from, to }) = threat_shift {
            let turn = context.turn.as_ref()?;
            context.facts["attitude_toward_hero"] = json!(turn.standing.attitude.label());
            eprintln!(
                "SOCIAL attitude npc=\"{}\" from={} to={} cause=threat",
                turn.dossier.npc_id,
                from.label(),
                to.label(),
            );
            log.push(
                &context.speaker,
                rpc::TableRole::Npc,
                0,
                rpc::TableKind::Action,
                &shift_line(shift, &context.hero_name),
            );
        }
        let modeled = self.model_dialogue(&context).await;
        let turn = context.turn.as_ref()?;
        let (text, fallback) = match modeled {
            // Jev (or the keyword fallback) decided before the reaction and reply.
            Ok((reply, _)) => (reply, None),
            Err(reason) => {
                // One server line per fallback, closed vocabulary, never reply text: the
                // session log is where a coordinator reads why a prepared line spoke.
                // (DIALOGUE-001 requirement; the OTEL event below carries the same reason.)
                let detail = match reason {
                    FallbackReason::RequestInvalid(label) => format!(" detail={label}"),
                    _ => String::new(),
                };
                eprintln!(
                    "NPC REPLY fallback reason={} npc={}{detail}",
                    reason.label(),
                    turn.dossier.npc_id,
                );
                let (answer, answer_key) = npc_dossier::fallback(
                    &turn.dossier,
                    turn.standing,
                    &context.typed_words,
                    approach,
                    &turn.withheld,
                    &npc_dossier::Memory {
                        conversation: &turn.conversation,
                        said: &turn.said,
                        offered: &turn.offered,
                    },
                );
                // A prepared, confided or curt answer counts as said under its memory key.
                if let Some(key) = answer_key {
                    turn.ledger.remember_offered(
                        context.member,
                        &turn.dossier,
                        std::slice::from_ref(&key),
                    );
                }
                (answer, Some(reason))
            }
        };
        let from_model = fallback.is_none();
        // What was whispered to this hero alone is answered to this hero alone: the table sees
        // that the speaker murmured, not the words.
        let private = !turn.withheld.released().is_empty();
        if private {
            log.push(
                &context.speaker,
                rpc::TableRole::Npc,
                0,
                rpc::TableKind::Action,
                &format!("murmurs to {}", context.hero_name),
            );
        } else {
            log.push(
                &context.speaker,
                rpc::TableRole::Npc,
                0,
                rpc::TableKind::Speech,
                &text,
            );
        }
        let recorded = turn.ledger.record_exchange(
            context.member,
            &turn.dossier,
            &turn.scene,
            approach,
            &context.typed_words,
            &text,
        );
        // Only the lines the reply actually used count as said; one it passed over is offered again.
        let used: Vec<String> = turn
            .offered
            .iter()
            .filter(|line| coverage(&line.text, &text) >= USED_COVERAGE)
            .map(|line| line.key.clone())
            .collect();
        turn.ledger
            .remember_offered(context.member, &turn.dossier, &used);
        if let Some(shift) = recorded.shift {
            log.push(
                &context.speaker,
                rpc::TableRole::Npc,
                0,
                rpc::TableKind::Action,
                &shift_line(shift, &context.hero_name),
            );
        }
        let ledger = turn.ledger.clone();
        let saved = tokio::task::spawn_blocking(move || ledger.save()).await;
        if !matches!(saved, Ok(Ok(()))) {
            let operation = df_observe::OperationContext {
                trace_parent: String::new(),
                build: crate::BUILD_ID.to_owned(),
            };
            let mut span = df_observe::begin(&operation, "gameplay.social.save");
            span.event(
                "social.save_failed",
                &[("social.error", format!("{saved:?}"))],
            );
            span.finish_unmeasured("save_failed");
        }
        Some(Exchange {
            text,
            approach,
            from_model,
            fallback,
            shift: recorded.shift,
            private,
        })
    }

    /// The display version of the hero's line (see `line_cleanup`): the model's tidy-up when it
    /// passes validation within its short deadline, else the deterministic normaliser.
    pub(super) async fn display_line(&self, context: &NpcReplyContext) -> String {
        super::line_cleanup::display_line(
            &self.storyteller,
            &context.typed_words,
            &context.known_names,
            content_rating(),
        )
        .await
        .text
    }

    /// The text model's in-character reply and its reading of the hero's approach, or why
    /// no model reply speaks: the storyteller is off, the member is over its bounds, the
    /// call failed, or the reply broke a rule on both attempts.
    async fn model_dialogue(
        &self,
        context: &NpcReplyContext,
    ) -> Result<(String, Option<Approach>), FallbackReason> {
        let turn = context.turn.as_ref().ok_or(FallbackReason::CallFailed)?;
        let rating = content_rating();
        if !self.storyteller.is_on() {
            return Err(FallbackReason::ModelOff);
        }
        let _admission = self
            .limiter
            .admit(context.member, Instant::now())
            .ok_or(FallbackReason::RateLimited)?;
        let slots = reply_slots(turn);
        let values = self
            .storyteller
            .express(&Expression {
                purpose: "npc_reply",
                facts: &context.facts,
                slots: &slots,
                style: npc_dossier::STYLE,
                profile: npc_dossier::profile(rating),
            })
            .await
            .map_err(|error| match error {
                StorytellerError::Off => FallbackReason::ModelOff,
                StorytellerError::BudgetExhausted => FallbackReason::BudgetExhausted,
                StorytellerError::Rejected(_) => FallbackReason::ReplyRejected,
                StorytellerError::InvalidRequest(label) => FallbackReason::RequestInvalid(label),
                StorytellerError::DeadlineExceeded | StorytellerError::Provider(_) => {
                    FallbackReason::CallFailed
                }
            })?;
        let reply = values
            .get("reply")
            .map(|reply| strip_outer_quotes(reply))
            .filter(|reply| !reply.is_empty())
            .ok_or(FallbackReason::EmptyReply)?;
        let approach = values
            .get("approach")
            .and_then(|word| Approach::parse(word));
        Ok((reply, approach))
    }

    pub(super) async fn reply(&self, context: NpcReplyContext) -> Option<String> {
        let rating = content_rating();
        if !self.storyteller.is_on() {
            return None;
        }
        let _admission = self.limiter.admit(context.member, Instant::now())?;
        let slots = [Slot::new("reply", REPLY_CHARACTERS)];
        let values = self
            .storyteller
            .express(&Expression {
                purpose: "npc_reply",
                facts: &context.facts,
                slots: &slots,
                style: STYLE,
                profile: CallProfile::short_reply(rating),
            })
            .await
            .ok()?;
        let reply = strip_outer_quotes(values.get("reply")?);
        (!reply.is_empty()).then_some(reply)
    }
}

/// The two slots of an NPC reply: the line spoken, which must pass the speaking rules and avoid
/// every withheld term and the speaker's stock phrases, and the reading of the hero's approach.
pub(super) fn reply_slots(turn: &Turn) -> [Slot; 2] {
    let mut refused = turn.withheld.all().to_vec();
    refused.extend(npc_dossier::style_phrases(&turn.dossier));
    [
        Slot::new("reply", npc_dossier::REPLY_CHARACTERS)
            .withholding(refused)
            .speaking(npc_dossier::spoken_rules(
                turn.answer_offered,
                &turn.said,
                turn.conversation.admitted_ignorance,
            )),
        Slot::new("approach", npc_dossier::APPROACH_CHARACTERS),
    ]
}

/// Models like to wrap a spoken line in quotation marks; the bubble shows the words only.
fn strip_outer_quotes(text: &str) -> String {
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
            return inner.trim().to_owned();
        }
    }
    text.to_owned()
}

#[cfg(test)]
mod tests {
    use super::super::adventure::dossier::Attitude;
    use super::*;

    const CLUE: &str = "The courier quietly tells you: the sealed packet is addressed to Vell at the Harbor Inn. Keep its destination between you.";

    /// A pack from before the explicit lists: only the clue text is known.
    #[test]
    fn the_names_the_scenes_speak_aloud_are_not_withheld_but_the_secret_phrase_still_is() {
        let story = StoryFacts {
            clue: CLUE.to_owned(),
            secret_terms: vec![
                "Net Loft".to_owned(),
                "Jorun".to_owned(),
                "false trail".to_owned(),
            ],
            public_names: vec!["Jorun".to_owned(), "Net Loft".to_owned()],
            speakers: Vec::new(),
            scene_lines: Vec::new(),
        };
        assert_eq!(
            unheard_secret_terms(&story, "A quiet room."),
            vec!["false trail".to_owned()]
        );
    }

    fn story() -> StoryFacts {
        StoryFacts {
            clue: CLUE.to_owned(),
            secret_terms: Vec::new(),
            public_names: Vec::new(),
            speakers: Vec::new(),
            scene_lines: Vec::new(),
        }
    }

    /// A generated pack: explicit secret terms and a profile for each speaker.
    fn listed_story() -> StoryFacts {
        StoryFacts {
            clue: CLUE.to_owned(),
            secret_terms: vec!["Vell".to_owned(), "Harbor Inn".to_owned()],
            public_names: Vec::new(),
            speakers: vec![
                SpeakerProfile {
                    name: "Courier".to_owned(),
                    gender: None,
                    age: None,
                    role: "a frightened courier carrying a sealed packet".to_owned(),
                    voice: "hurried and anxious".to_owned(),
                },
                SpeakerProfile {
                    name: "Bandit".to_owned(),
                    gender: None,
                    age: None,
                    role: "a hostile bandit fighting the heroes".to_owned(),
                    voice: "sneering and rough".to_owned(),
                },
            ],
            scene_lines: Vec::new(),
        }
    }

    fn member(byte: u8) -> MemberId {
        MemberId::from_bytes(&[byte; 16]).unwrap()
    }

    fn view(clue: &str, speaker: &str) -> rpc::ViewMessage {
        rpc::ViewMessage {
            revision: None,
            audience: Some(rpc::view_message::Audience::Player(
                rpc::PlayerGameplayView {
                    narration: "You reach the Lantern Wharf in the rain.".to_owned(),
                    private_clue: clue.to_owned(),
                    scene: Some(rpc::GameplayScene {
                        title: "The Broken Seal".to_owned(),
                        ..Default::default()
                    }),
                    offers: vec![rpc::GameplayActionOffer {
                        label: "Ask the courier what he knows".to_owned(),
                        ..Default::default()
                    }],
                    journey: Some(rpc::JourneyView {
                        speaker: speaker.to_owned(),
                        dialogue: "Please. I cannot do this alone.".to_owned(),
                        own_character: Some(rpc::CharacterSheet {
                            name: "Brannoch".to_owned(),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )),
        }
    }

    #[test]
    fn dossier_reply_facts_include_current_narration_and_cast() {
        let adventure = super::super::npc_dossier_tests::adventure(1);
        let patron = adventure
            .dossiers
            .iter()
            .find(|dossier| dossier.role == super::super::adventure::dossier::NpcRole::Patron)
            .unwrap();
        let social = SocialLedger::open(adventure.dossiers.clone(), "scene-facts", None).unwrap();
        let mut projected = view("", "Perrin");
        let Some(rpc::view_message::Audience::Player(player)) = projected.audience.as_mut() else {
            unreachable!();
        };
        player.narration = "Calla takes the ledger. Perrin lets out a long breath.".to_owned();
        player.scene.as_mut().unwrap().cast = vec![rpc::SceneActor {
            id: patron.npc_id.strip_prefix("patron:").unwrap().to_owned(),
            name: "Perrin".to_owned(),
            role: rpc::SceneRole::Patron as i32,
            ..Default::default()
        }];

        let context = NpcReplyContext::for_cast(
            &projected,
            member(1),
            "What just happened, Perrin?",
            &story(),
            &social,
            "Opening",
        )
        .unwrap();

        assert_eq!(
            context.facts["now"]["narration"],
            "Calla takes the ledger. Perrin lets out a long breath."
        );
        assert_eq!(
            context.facts["now"]["characters_present"][0]["name"],
            "Perrin"
        );
        assert_eq!(
            context.facts["now"]["characters_present"][0]["role"],
            "Patron"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_threat_lowers_standing_before_the_model_generates_its_reply() {
        let endpoint = FakeEndpoint::start(vec![Reply::Output(json!({
            "reply": "Put that away. I will not be threatened.",
            "approach": "question"
        }))]);
        let (voice, cache) = voice(&endpoint, "voice-threat-attitude");
        let mut adventure = super::super::npc_dossier_tests::adventure(1);
        let patron = adventure
            .dossiers
            .iter_mut()
            .find(|dossier| dossier.role == NpcRole::Patron)
            .unwrap();
        patron.starting_attitude = Attitude::Friendly;
        let patron = patron.clone();
        let social = SocialLedger::open(adventure.dossiers, "threat", None).unwrap();
        let mut projected = view("", "Perrin");
        let Some(rpc::view_message::Audience::Player(player)) = projected.audience.as_mut() else {
            unreachable!();
        };
        player.scene.as_mut().unwrap().cast = vec![rpc::SceneActor {
            id: patron.npc_id.strip_prefix("patron:").unwrap().to_owned(),
            name: patron.name.clone(),
            role: rpc::SceneRole::Patron as i32,
            speaking: true,
            ..Default::default()
        }];
        let context = NpcReplyContext::for_cast(
            &projected,
            member(1),
            "Hand over the key or else.",
            &story(),
            &social,
            "Opening",
        )
        .unwrap();
        let log = TableLog::new();

        voice.converse(context, &log).await;

        assert_eq!(
            social.standing(member(1), &patron).attitude,
            Attitude::Indifferent
        );
        assert!(
            endpoint.bodies()[0]["input"]
                .as_str()
                .unwrap()
                .contains("\"attitude_toward_hero\":\"indifferent\"")
        );
        assert!(
            log.lines()
                .iter()
                .any(|line| line.text == "cools toward Brannoch")
        );
        std::fs::remove_dir_all(cache).unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_threat_to_a_contact_lowers_standing_before_the_model_generates_its_reply() {
        let endpoint = FakeEndpoint::start(vec![Reply::Output(json!({
            "reply": "Keep your voice down. Dangerous people are listening.",
            "approach": "question"
        }))]);
        let (voice, cache) = voice(&endpoint, "voice-contact-threat-attitude");
        let mut adventure = super::super::npc_dossier_tests::adventure(1);
        let contact = adventure
            .dossiers
            .iter_mut()
            .find(|dossier| dossier.role == NpcRole::Contact)
            .unwrap();
        contact.starting_attitude = Attitude::Indifferent;
        let contact = contact.clone();
        let social = SocialLedger::open(adventure.dossiers, "contact-threat", None).unwrap();
        let mut projected = view("", "Perrin");
        let Some(rpc::view_message::Audience::Player(player)) = projected.audience.as_mut() else {
            unreachable!();
        };
        player.scene.as_mut().unwrap().cast = vec![rpc::SceneActor {
            id: contact.npc_id.strip_prefix("contact:").unwrap().to_owned(),
            name: contact.name.clone(),
            role: rpc::SceneRole::Contact as i32,
            speaking: true,
            ..Default::default()
        }];
        let context = NpcReplyContext::for_cast(
            &projected,
            member(1),
            "Hand over the coin you owe us or I will break your arm.",
            &story(),
            &social,
            "Opening",
        )
        .unwrap();
        let log = TableLog::new();

        voice.converse(context, &log).await;

        assert_eq!(
            social.standing(member(1), &contact).attitude,
            Attitude::Hostile
        );
        assert!(
            endpoint.bodies()[0]["input"]
                .as_str()
                .unwrap()
                .contains("\"attitude_toward_hero\":\"hostile\"")
        );
        assert!(
            log.lines()
                .iter()
                .any(|line| line.text == "cools toward Brannoch")
        );
        std::fs::remove_dir_all(cache).unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_paraphrased_private_clue_reply_is_returned_to_hero_but_not_the_table() {
        const PARAPHRASE: &str = "It is meant for the person waiting at the waterfront lodging house. Keep its destination between us.";
        let endpoint = FakeEndpoint::start(vec![Reply::Output(json!({
            "reply": PARAPHRASE,
            "approach": "question"
        }))]);
        let (voice, cache) = voice(&endpoint, "voice-private-clue-paraphrase");
        let adventure = super::super::npc_dossier_tests::adventure(1);
        let patron = adventure
            .dossiers
            .iter()
            .find(|dossier| dossier.role == NpcRole::Patron)
            .unwrap();
        let social = SocialLedger::open(adventure.dossiers.clone(), "private-clue", None).unwrap();
        let mut projected = view(
            "The sealed packet is addressed to Vell at the Harbor Inn. Keep its destination between you.",
            "Perrin",
        );
        let Some(rpc::view_message::Audience::Player(player)) = projected.audience.as_mut() else {
            unreachable!();
        };
        player.scene.as_mut().unwrap().cast = vec![rpc::SceneActor {
            id: patron.npc_id.strip_prefix("patron:").unwrap().to_owned(),
            name: patron.name.clone(),
            role: rpc::SceneRole::Patron as i32,
            speaking: true,
            ..Default::default()
        }];
        let mut story = listed_story();
        story.clue = format!(
            "{} tells you the sealed packet is addressed to Vell at the Harbor Inn. Keep its destination between you.",
            patron.name
        );
        let context = NpcReplyContext::for_cast(
            &projected,
            member(1),
            "Where is the packet going?",
            &story,
            &social,
            "Opening",
        )
        .unwrap();
        assert!(
            !context
                .turn
                .as_ref()
                .unwrap()
                .withheld
                .released()
                .is_empty()
        );
        let log = TableLog::new();

        let response = voice.converse(context, &log).await;

        assert_eq!(response.as_deref(), Some(PARAPHRASE));
        let lines = log.lines();
        assert!(lines.iter().any(|line| {
            line.kind == rpc::TableKind::Action as i32 && line.text == "murmurs to Brannoch"
        }));
        assert!(
            !lines.iter().any(|line| {
                line.kind == rpc::TableKind::Speech as i32 && line.text == PARAPHRASE
            })
        );
        std::fs::remove_dir_all(cache).unwrap();
    }

    #[test]
    fn a_player_without_the_clue_gets_its_key_terms_forbidden_and_not_its_text() {
        let context = NpcReplyContext::from_view(
            &view("", "Courier"),
            member(1),
            "Where is it going?",
            &story(),
        )
        .unwrap();

        let facts = &context.facts;
        assert_eq!(facts["player"]["exact_words"], "Where is it going?");
        assert_eq!(facts["player"]["hero_name"], "Brannoch");
        assert_eq!(facts["npc"]["name"], "Courier");
        assert_eq!(
            facts["npc_current_scripted_line"],
            "Please. I cannot do this alone."
        );
        assert_eq!(facts["offered_choices"], json!([] as [String; 0]));
        assert_eq!(facts["forbidden"], json!(["Vell", "Harbor Inn", "Inn"]));
        assert!(
            facts
                .get("secret_clue_this_player_was_privately_told_and_may_be_reminded_of")
                .is_none()
        );
        assert!(!facts.to_string().contains("addressed to"));
    }

    #[test]
    fn terms_the_whole_room_already_sees_are_not_forbidden() {
        let mut shared = view("", "Courier");
        let Some(rpc::view_message::Audience::Player(player)) = shared.audience.as_mut() else {
            unreachable!();
        };
        player.narration = "Rain beats on the harbor. Vell is waiting.".to_owned();

        let context = NpcReplyContext::from_view(&shared, member(1), "Who?", &story()).unwrap();

        assert_eq!(context.facts["forbidden"], json!(["Harbor Inn", "Inn"]));
    }

    #[test]
    fn a_player_who_was_told_the_clue_may_hear_it_again() {
        let told = view(
            "The packet is addressed to Vell at the Harbor Inn.",
            "Courier",
        );

        let context =
            NpcReplyContext::from_view(&told, member(1), "Where exactly?", &story()).unwrap();

        // Table talk is public: the secret's terms stay forbidden for the asker who holds it.
        assert_eq!(
            context.facts["forbidden"],
            json!(["Vell", "Harbor Inn", "Inn"])
        );
        assert_eq!(
            context.facts["secret_clue_this_player_was_privately_told"],
            "The packet is addressed to Vell at the Harbor Inn."
        );
    }

    #[test]
    fn explicit_secret_terms_replace_the_guess_and_the_speakers_role_and_voice_are_sent() {
        let context = NpcReplyContext::from_view(
            &view("", "Courier"),
            member(1),
            "Where is it going?",
            &listed_story(),
        )
        .unwrap();

        let facts = &context.facts;
        assert_eq!(facts["forbidden"], json!(["Vell", "Harbor Inn"]));
        assert_eq!(
            facts["npc"],
            json!({
                "name": "Courier",
                "role": "a frightened courier carrying a sealed packet",
                "voice": "hurried and anxious",
                "age_group": "not stated",
                "based_on_real_person": false,
                "sexual_content_allowed": false,
            })
        );
        assert!(
            facts["listener_knowledge"]
                .as_str()
                .unwrap()
                .contains("NOT been told")
        );
    }

    #[test]
    fn a_hero_who_holds_the_secret_is_told_so_plainly() {
        let told = view(
            "The packet is addressed to Vell at the Harbor Inn.",
            "Courier",
        );

        let context =
            NpcReplyContext::from_view(&told, member(1), "Where exactly?", &listed_story())
                .unwrap();

        assert!(
            context.facts["listener_knowledge"]
                .as_str()
                .unwrap()
                .contains("others are listening, so allude to it without stating it")
        );
        assert_eq!(context.facts["forbidden"], json!(["Vell", "Harbor Inn"]));
    }

    #[test]
    fn the_enemy_speaking_in_the_confrontation_answers_as_the_enemy() {
        let context = NpcReplyContext::from_view(
            &view("", "Bandit"),
            member(1),
            "Surrender!",
            &listed_story(),
        )
        .unwrap();

        assert_eq!(context.facts["npc"]["name"], "Bandit");
        assert_eq!(
            context.facts["npc"]["role"],
            "a hostile bandit fighting the heroes"
        );
        assert_eq!(context.facts["npc"]["voice"], "sneering and rough");
    }

    #[test]
    fn nobody_answers_when_no_one_is_speaking_or_the_text_is_empty() {
        assert!(NpcReplyContext::from_view(&view("", ""), member(1), "Hello", &story()).is_none());
        assert!(
            NpcReplyContext::from_view(&view("", "Courier"), member(1), " \n ", &story()).is_none()
        );
    }

    #[test]
    fn typed_words_lose_control_characters_and_are_cut_to_five_hundred_characters() {
        let long = format!("Hello\u{0}there\n{}", "a".repeat(900));

        let words = sanitize_player_words(&long);

        assert_eq!(words.chars().count(), 500);
        assert!(words.starts_with("Hello there "));
        assert!(!words.chars().any(char::is_control));
    }

    #[test]
    fn a_member_has_one_reply_in_flight_and_six_a_minute() {
        let limiter = Arc::new(ReplyLimiter::default());
        let start = Instant::now();
        let (one, two) = (member(1), member(2));

        let held = limiter.admit(one, start).unwrap();
        assert!(limiter.admit(one, start).is_none(), "second in flight");
        assert!(
            limiter.admit(two, start).is_some(),
            "other members are independent"
        );
        drop(held);
        for step in 1..6 {
            drop(
                limiter
                    .admit(one, start + Duration::from_secs(step))
                    .unwrap(),
            );
        }
        assert!(
            limiter
                .admit(one, start + Duration::from_secs(10))
                .is_none(),
            "seventh within a minute"
        );
        assert!(
            limiter
                .admit(one, start + Duration::from_secs(61))
                .is_some()
        );
    }

    #[test]
    fn outer_quotation_marks_are_removed_from_a_spoken_line() {
        assert_eq!(
            strip_outer_quotes("\u{201c}I cannot say.\u{201d}"),
            "I cannot say."
        );
        assert_eq!(strip_outer_quotes("\"Quiet now.\""), "Quiet now.");
        assert_eq!(
            strip_outer_quotes("\"One,\" he said, \"two.\""),
            "\"One,\" he said, \"two.\""
        );
        assert_eq!(strip_outer_quotes("  Plain.  "), "Plain.");
    }

    use super::super::storyteller::tests::{FakeEndpoint, Reply, cache_directory, teller};

    fn voice(endpoint: &FakeEndpoint, name: &str) -> (NpcVoice, std::path::PathBuf) {
        let cache = cache_directory(name);
        let storyteller = teller(endpoint.address, 20_000, Duration::from_secs(5), &cache);
        (NpcVoice::new(Arc::new(storyteller)), cache)
    }

    fn question(words: &str) -> NpcReplyContext {
        NpcReplyContext::from_view(&view("", "Courier"), member(1), words, &story()).unwrap()
    }

    mod approach_model {
        include!("approach_model_reply_tests.rs");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_speaker_answers_in_character_and_the_model_sees_the_secret_only_as_forbidden() {
        let endpoint = FakeEndpoint::start(vec![Reply::Output(json!({
            "reply": "\u{201c}I wish I knew, friend. Keep your voice low.\u{201d}"
        }))]);
        let (voice, cache) = voice(&endpoint, "voice-success");

        let reply = voice.reply(question("Who is following you?")).await;

        assert_eq!(
            reply.as_deref(),
            Some("I wish I knew, friend. Keep your voice low.")
        );
        let sent = endpoint.bodies()[0]["input"].as_str().unwrap().to_owned();
        assert!(sent.contains("Who is following you?"));
        assert!(sent.contains("\"forbidden\":[\"Vell\",\"Harbor Inn\",\"Inn\"]"));
        assert!(!sent.contains("addressed to"));
        std::fs::remove_dir_all(cache).unwrap();
    }

    fn holder_question(words: &str) -> NpcReplyContext {
        let told = view(
            "The packet is addressed to Vell at the Harbor Inn. Keep its destination between you.",
            "Courier",
        );
        NpcReplyContext::from_view(&told, member(1), words, &listed_story()).unwrap()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_scripted_conversation_reaches_the_table_log_in_order_without_the_secret() {
        use super::super::table_log::{TableLog, action_lines};
        let endpoint = FakeEndpoint::start(vec![
            // A reply that states the secret is refused whole, twice.
            Reply::Output(json!({"reply": "It goes to Vell at the Harbor Inn, friend."})),
            Reply::Output(json!({"reply": "Vell waits at the Harbor Inn."})),
            // A reply that alludes to it is delivered.
            Reply::Output(json!({"reply": "You know where it must go. Keep your voice down."})),
            Reply::Status(500, json!({"error": {}})),
        ]);
        let (voice, cache) = voice(&endpoint, "voice-table-log");
        let log = TableLog::new();
        for line in action_lines(
            rpc::GameplayActionKind::AskCourier,
            "Brannoch",
            &[],
            "greatsword",
            "",
            "Ask the courier what he knows",
        ) {
            log.push(
                &line.speaker,
                rpc::TableRole::Hero,
                0,
                rpc::TableKind::Action,
                &line.text,
            );
        }

        // The holder asks; the model leaks, so only the player's own line is public.
        let first = holder_question("Where exactly does it go?");
        first.publish_typed_line(&log, &first.typed_words);
        assert_eq!(voice.reply_in_public(first, &log).await, None);
        // A second question is answered discreetly.
        let second = holder_question("You know. Tell me quietly.");
        second.publish_typed_line(&log, &second.typed_words);
        let reply = voice.reply_in_public(second, &log).await;
        assert_eq!(
            reply.as_deref(),
            Some("You know where it must go. Keep your voice down.")
        );
        // A failing model adds only the player's line.
        let third = holder_question("And then?");
        third.publish_typed_line(&log, &third.typed_words);
        assert_eq!(voice.reply_in_public(third, &log).await, None);

        let lines = log.lines();
        let shown: Vec<(&str, i32, i32, &str)> = lines
            .iter()
            .map(|line| {
                (
                    line.speaker.as_str(),
                    line.role,
                    line.kind,
                    line.text.as_str(),
                )
            })
            .collect();
        assert_eq!(
            shown,
            [
                (
                    "Brannoch",
                    rpc::TableRole::Hero as i32,
                    rpc::TableKind::Action as i32,
                    "asks the courier what he knows"
                ),
                (
                    "Brannoch",
                    rpc::TableRole::Hero as i32,
                    rpc::TableKind::Speech as i32,
                    "Where exactly does it go?"
                ),
                (
                    "Brannoch",
                    rpc::TableRole::Hero as i32,
                    rpc::TableKind::Speech as i32,
                    "You know. Tell me quietly."
                ),
                (
                    "Courier",
                    rpc::TableRole::Npc as i32,
                    rpc::TableKind::Speech as i32,
                    "You know where it must go. Keep your voice down."
                ),
                (
                    "Brannoch",
                    rpc::TableRole::Hero as i32,
                    rpc::TableKind::Speech as i32,
                    "And then?"
                ),
            ]
        );
        assert!(
            lines
                .windows(2)
                .all(|pair| pair[0].sequence < pair[1].sequence)
        );
        let everything: String = lines.iter().map(|line| line.text.to_lowercase()).collect();
        for hidden in ["vell", "harbor inn", "addressed to", "destination"] {
            assert!(!everything.contains(hidden), "{hidden} reached the table");
        }
        // The model was told the secret's terms are forbidden even though the asker holds it.
        let sent = endpoint.bodies()[0]["input"].as_str().unwrap().to_owned();
        assert!(sent.contains("\"forbidden\":[\"Vell\",\"Harbor Inn\"]"));
        assert!(sent.contains("allude to it without stating it"));
        std::fs::remove_dir_all(cache).unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_reply_asks_for_low_reasoning_and_a_cap_of_three_hundred_tokens() {
        let endpoint = FakeEndpoint::start(vec![Reply::Output(json!({"reply": "Quiet now."}))]);
        let (voice, cache) = voice(&endpoint, "voice-effort");

        voice.reply(question("Who is following you?")).await;

        let body = &endpoint.bodies()[0];
        assert_eq!(body["reasoning"], json!({"effort": "low"}));
        assert_eq!(body["max_output_tokens"], 300);
        std::fs::remove_dir_all(cache).unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_reply_that_leaks_the_secret_is_never_delivered() {
        let leak = Reply::Output(json!({"reply": "It goes to the Harbor Inn, quietly."}));
        let endpoint = FakeEndpoint::start(vec![
            leak,
            Reply::Output(json!({"reply": "Ask Vell, if you must."})),
        ]);
        let (voice, _cache) = voice(&endpoint, "voice-leak");

        let reply = voice.reply(question("Where is the packet going?")).await;

        assert_eq!(reply, None);
        assert_eq!(endpoint.connections(), 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_failing_model_gives_no_reply_so_the_response_stays_as_before() {
        let endpoint = FakeEndpoint::start(vec![Reply::Status(500, json!({"error": {}}))]);
        let (voice, _cache) = voice(&endpoint, "voice-failure");

        assert_eq!(voice.reply(question("Hello?")).await, None);
        assert_eq!(NpcVoice::silent().reply(question("Hello?")).await, None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_seventh_reply_within_a_minute_is_refused_without_a_call() {
        let endpoint = FakeEndpoint::start(vec![Reply::Output(json!({"reply": "Quiet now."}))]);
        let (voice, cache) = voice(&endpoint, "voice-rate");

        for _ in 0..6 {
            assert_eq!(
                voice.reply(question("Hello?")).await.as_deref(),
                Some("Quiet now.")
            );
        }
        let seventh = voice.reply(question("Hello?")).await;

        assert_eq!(seventh, None);
        assert_eq!(
            endpoint.connections(),
            1,
            "the repeats were served from the cache"
        );
        std::fs::remove_dir_all(cache).unwrap();
    }
}
