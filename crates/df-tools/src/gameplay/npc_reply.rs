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

use std::sync::Arc;

use df_protocol::common as rpc;
use df_types::MemberId;
use serde_json::{Value, json};

use super::{
    adventure::dossier::{Attitude, Dossier, NpcRole, QuestionSubject, mentions_term},
    approach_model::ApproachModel,
    npc_dossier::{self, Clue, Inputs, Turn, Withheld, released_terms},
    pack::{Age, PackFile, SceneKey, SpeakerProfile},
    social::{
        Approach, EmotionLevel, EmotionReaction, InfluenceWillingness, Shift, SocialLedger,
        shift_line,
    },
    speech_text::{coverage, sentences},
    startup_configuration::content_rating,
    storyteller::{
        CallProfile, Expression, Slot, Storyteller, StorytellerError, ValidationFailure, age_group,
        contains_whole_word, sexual_content_allowed,
    },
    table_log::TableLog,
};

#[path = "npc_answer.rs"]
mod answer;
#[path = "npc_room.rs"]
mod room;

/// Longest in-character reply, in characters.
pub(super) const REPLY_CHARACTERS: usize = 220;
/// Only this much of the player's words is shown to the model.
const MAXIMUM_PLAYER_CHARACTERS: usize = 500;
/// How much of an offered line the reply must say for the line to count as said.
const USED_COVERAGE: f64 = 0.45;

const STYLE: &str = "Speak in first person as facts.npc, using its role and voice. No quotation \
marks, colon or semicolon. Brief plain words, one thought at a time. Deflect forbidden or hidden \
facts; allude quietly to a secret the hero already knows. Enemies taunt, threaten or bargain, \
never help. Never promise mechanical results or name menus, choices, clicks or what to ask. \
Honor your_previous_public_lines as your own spoken knowledge; do not contradict them or invent \
a revelation.";

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
    scenes: Vec<super::pack::Scene>,
}

impl StoryFacts {
    pub(super) fn from_pack(pack: &PackFile) -> Self {
        Self {
            clue: pack.courier_clue.clone(),
            secret_terms: pack.secret_terms.clone(),
            public_names: spoken_names(pack),
            speakers: pack.speakers.clone(),
            scenes: pack.scenes.clone(),
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

fn asks_about_private_clue(words: &str) -> bool {
    let lowered = words.to_lowercase();
    [
        "clue",
        "whisper",
        "what did you tell me",
        "what did you say to me",
        "what you told me quietly",
        "tell me quietly",
    ]
    .iter()
    .any(|phrase| contains_whole_word(&lowered, phrase))
}

fn private_question(words: &str, story: &StoryFacts) -> bool {
    let lowered = words.to_lowercase();
    let destination_question = [
        "where is the packet going",
        "where does the packet go",
        "where do i take",
        "where do we take",
        "where should i take",
        "where should we take",
        "where to deliver",
        "its destination",
        "where exactly",
    ]
    .iter()
    .any(|phrase| contains_whole_word(&lowered, phrase));
    asks_about_private_clue(words)
        || destination_question
        || story
            .secret_terms
            .iter()
            .filter(|term| {
                !story
                    .public_names
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(term))
            })
            .any(|term| mentions_term(words, term))
}

fn private_clue_refusal(turn: &Turn) -> (String, String) {
    const LINES: [&str; 4] = [
        "That whisper was meant for someone else. I won't repeat it.",
        "I shared that confidence with another hero, and it stays with them.",
        "What I said in private belongs to the person I told.",
        "I won't pass another person's confidence to you.",
    ];
    let said: std::collections::BTreeSet<String> = turn
        .said
        .iter()
        .flat_map(|line| sentences(line))
        .map(|sentence| super::speech_text::sentence_key(&sentence))
        .collect();
    LINES
        .iter()
        .enumerate()
        .find(|(index, line)| {
            let key = format!("private_clue:withheld:{index}");
            !turn.conversation.voiced.iter().any(|voiced| voiced == &key)
                && !said.contains(&super::speech_text::sentence_key(line))
        })
        .map(|(index, line)| (line.to_string(), format!("private_clue:withheld:{index}")))
        .unwrap_or_else(|| {
            (
                "I have already told you that confidence stays with its owner.".to_owned(),
                "private_clue:withheld:repeat".to_owned(),
            )
        })
}

fn revision_key(view: &rpc::ViewMessage) -> String {
    let Some(revision) = view.revision.as_ref() else {
        return "0:0".to_owned();
    };
    format!(
        "{}:{}",
        revision
            .epoch
            .as_ref()
            .and_then(|epoch| epoch.value)
            .unwrap_or(0),
        revision.sequence.unwrap_or(0),
    )
}

/// Everything one reply needs, captured by the actor and owned so the model call can run
/// elsewhere.
pub(super) struct NpcReplyContext {
    member: MemberId,
    pub(super) scene_identity: String,
    pub(super) npc_id: String,
    pub(super) hero_id: String,
    /// The unique live quote this exact turn may answer with an acceptance, decline or
    /// counteroffer. Captured by the authoritative actor after source-line validation.
    pub(super) pending_quote: Option<super::journey::living_scene::SceneTradeQuote>,
    publication_guard: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    pub(super) facts: Value,
    pub(super) speaker: String,
    pub(super) actor_id: String,
    hero_name: String,
    hero_index: u32,
    pub(super) typed_words: String,
    public_scene: String,
    /// Names present in the scene (heroes, cast, places): the only names a cleaned-up line may
    /// be corrected to.
    known_names: Vec<String>,
    /// The public appearance facts the hero's own sheet lets another character perceive.
    hero_visible: Value,
    identity_reply: Option<String>,
    withheld_subjects: Vec<QuestionSubject>,
    private_reply: bool,
    private_terms: Vec<String>,
    room: Option<super::adventure::scene_model::SceneNow>,
    /// The session basis observed when this reply was routed.  Event proposals are stale when
    /// the owner commits against any other basis.
    pub(super) observed_revision: String,
    /// Set when the addressed NPC has a backstory: the reply then draws on it and moves the
    /// NPC's attitude toward this hero.
    pub(super) turn: Option<Box<Turn>>,
}

/// How one exchange with an NPC that has a backstory went.
pub(super) struct Exchange {
    pub(super) text: String,
    /// Public NPC speech line sequence, when a line was actually shown in the table log.
    pub(super) source_line_sequence: Option<u64>,
    pub(super) outcome: ExchangeOutcome,
    /// What the hero's words did, as the model or the keyword reader read them.
    pub(super) approach: Approach,
    /// False when the dossier's prepared answer replied instead of the text model.
    pub(super) from_model: bool,
    /// Why the model reply never spoke, when it did not. Closed vocabulary, never reply text.
    pub(super) fallback: Option<FallbackReason>,
    pub(super) shift: Option<Shift>,
    /// The reply spoke of what was whispered to this hero alone, so only this hero was shown it.
    pub(super) private: bool,
    /// A Jev proposal for the owner; this is never an authoritative scene mutation.
    pub(super) event_proposal: Option<super::npc_event_classifier::Proposal>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ExchangeOutcome {
    Spoken,
    Withheld,
    Unavailable,
}

impl ExchangeOutcome {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Spoken => "spoken",
            Self::Withheld => "withheld",
            Self::Unavailable => "unavailable",
        }
    }
}

impl Exchange {
    fn observe(&self) {
        let operation = df_observe::OperationContext {
            trace_parent: String::new(),
            build: crate::BUILD_ID.to_owned(),
        };
        let mut span = df_observe::begin(&operation, "gameplay.social.exchange");
        span.event(
            "social.exchange",
            &[
                ("social.approach", self.approach.label().to_owned()),
                (
                    "social.source",
                    if self.outcome == ExchangeOutcome::Unavailable {
                        "unavailable"
                    } else if self.outcome == ExchangeOutcome::Withheld {
                        "withheld"
                    } else if self.from_model {
                        "model"
                    } else {
                        "prepared"
                    }
                    .to_owned(),
                ),
                ("social.shift", format!("{:?}", self.shift)),
                ("social.private", self.private.to_string()),
                ("social.outcome", self.outcome.label().to_owned()),
                (
                    "social.event_proposal",
                    self.event_proposal
                        .as_ref()
                        .map_or("none", |proposal| proposal.kind.label())
                        .to_owned(),
                ),
            ],
        );
        // One line per fallback, closed vocabulary, never reply text: today nothing records
        // why a prepared line spoke instead of the model.
        if let Some(reason) = self.fallback {
            span.event(
                "social.fallback",
                &[("social.reason", reason.label().to_owned())],
            );
        }
        span.finish_unmeasured(self.outcome.label());
    }
}

/// Why a model reply never spoke. Closed vocabulary for the one server line each fallback
/// prints; it never carries reply text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FallbackReason {
    /// The storyteller is off: no call was made.
    ModelOff,
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
            room::addresses(words, &actor.name)
                && (mentions_term(words, &dossier.name) || mentions_term(words, &actor.name))
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
    pub(super) fn watch_publication(
        &mut self,
        updates: tokio::sync::watch::Receiver<df_model::checkpoint::Checkpoint>,
    ) {
        let expected = self.scene_identity.clone();
        let member = self.member;
        let actor_id = self.actor_id.clone();
        self.publication_guard = Some(Arc::new(move || {
            let current = updates.borrow();
            let Ok(projection) = super::wire::project_journey(
                &current,
                df_persistence::local_demo_scope::LocalDemoRole::Player,
                member,
            ) else {
                return false;
            };
            let Some(rpc::view_message::Audience::Player(player)) = &projection.view.audience
            else {
                return false;
            };
            let Some(scene) = &player.scene else {
                return false;
            };
            format!("{}|{}", scene.title, scene.scene_asset) == expected
                && (actor_id.is_empty()
                    || super::npc_target::current(&scene.cast, &actor_id).is_some())
        }));
    }

    fn publication_current(&self) -> bool {
        let valid = self.publication_guard.as_ref().is_none_or(|guard| guard());
        if !valid {
            super::actor::record_audit_diagnostic(
                "gameplay.npc_target",
                "NPC reply publication suppressed: stale scene or actor".into(),
                serde_json::Map::from_iter([
                    ("resource_type".into(), json!("state")),
                    ("stage".into(), json!("npc_reply_publication")),
                    ("status".into(), json!("stale")),
                    ("actor_id".into(), json!(self.actor_id)),
                ]),
                &[
                    ("gameplay.resource.type", "state".into()),
                    ("routing.validation", "stale_publication".into()),
                ],
            );
        }
        valid
    }

    /// The context for a line typed to whoever on stage it addresses: an NPC with a dossier
    /// when the adventure has them, otherwise (and for packs without dossiers) the scene's
    /// single speaker as before. `scene` names the current scene for the attitude bounds.
    #[cfg(test)]
    pub(super) fn for_cast(
        view: &rpc::ViewMessage,
        member: MemberId,
        typed: &str,
        story: &StoryFacts,
        social: &Arc<SocialLedger>,
        scene: &str,
    ) -> Option<Self> {
        let cast = match &view.audience {
            Some(rpc::view_message::Audience::Player(player)) => player
                .scene
                .as_ref()
                .map(|scene| scene.cast.as_slice())
                .unwrap_or_default(),
            _ => return None,
        };
        let target = match super::npc_target::exact(typed, cast) {
            super::npc_target::Target::Actor(id) => Some(id),
            super::npc_target::Target::Description | super::npc_target::Target::PossibleAddress
                if social.is_active() =>
            {
                // This test adapter also exercises room aliases and ordinary compound speech.
                // The lexical cast precheck cannot resolve either; use the same dossier/room
                // constructor that production calls after its separate routing stage.
                return Self::with_dossier(view, member, typed, story, social, scene, None);
            }
            super::npc_target::Target::Ambiguous
            | super::npc_target::Target::Absent
            | super::npc_target::Target::Description
            | super::npc_target::Target::PossibleAddress => return None,
            super::npc_target::Target::Unspecified => None,
        };
        Self::for_target(view, member, typed, story, social, scene, target.as_deref())
    }

    pub(super) fn for_target(
        view: &rpc::ViewMessage,
        member: MemberId,
        typed: &str,
        story: &StoryFacts,
        social: &Arc<SocialLedger>,
        scene: &str,
        target: Option<&str>,
    ) -> Option<Self> {
        if let Some(id) = target {
            let Some(rpc::view_message::Audience::Player(player)) = &view.audience else {
                return None;
            };
            super::npc_target::current(&player.scene.as_ref()?.cast, id)?;
        }
        if social.is_active()
            && let Some(context) =
                Self::with_dossier(view, member, typed, story, social, scene, target)
        {
            return Some(context);
        }
        Self::from_view_target(view, member, typed, story, target)
    }

    fn with_dossier(
        view: &rpc::ViewMessage,
        member: MemberId,
        typed: &str,
        story: &StoryFacts,
        social: &Arc<SocialLedger>,
        scene_key: &str,
        target: Option<&str>,
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
        let room = social.scene_now(scene_key).or_else(|| {
            story
                .scenes
                .iter()
                .find(|scene| format!("{:?}", scene.key) == scene_key)
                .and_then(|scene| {
                    scene.setting.as_ref()?.model.as_ref().map(|model| {
                        super::adventure::scene_model::SceneNow {
                            model: model.for_scene(scene.key),
                            changes: Vec::new(),
                        }
                    })
                })
        });
        let background: Vec<rpc::SceneActor> = room
            .as_ref()
            .map(|room| {
                room.model
                    .people
                    .iter()
                    .filter(|person| person.is_background)
                    .map(|person| rpc::SceneActor {
                        id: person.id.clone(),
                        name: person.name.clone(),
                        role: rpc::SceneRole::Bystander as i32,
                        ..Default::default()
                    })
                    .collect()
            })
            .unwrap_or_default();
        let present: Vec<(&rpc::SceneActor, &Dossier)> = scene
            .cast
            .iter()
            .chain(&background)
            .filter_map(|actor| {
                let role = rpc::SceneRole::try_from(actor.role).ok()?;
                Some((actor, social.dossier_of(role, &actor.id)?))
            })
            .collect();
        let lowered = words.to_lowercase();
        let explicitly_named = present
            .iter()
            .find(|(actor, dossier)| {
                actor.role != rpc::SceneRole::Bystander as i32
                    && [&actor.name, &dossier.name].iter().any(|name| {
                        let name = name.to_lowercase();
                        [
                            format!("{name},"),
                            format!("{name}:"),
                            format!("i ask {name} "),
                            format!("i talk to {name} "),
                            format!("i walk over to {name} "),
                        ]
                        .iter()
                        .any(|prefix| lowered.starts_with(prefix))
                    })
            })
            .copied();
        let addressed = room
            .as_ref()
            .and_then(|room| room.model.resolve_target(&words))
            .filter(|target| target.kind == super::adventure::scene_model::SceneTargetKind::Person)
            .filter(|target| room::addresses(&words, &target.name))
            .and_then(|target| {
                present
                    .iter()
                    .find(|(actor, dossier)| {
                        target.ids.iter().any(|id| {
                            actor.id == *id
                                || dossier.npc_id.starts_with(&format!("{id}:"))
                                || dossier.npc_id.ends_with(&format!(":{id}"))
                        })
                    })
                    .copied()
            });
        let (actor, dossier) = if let Some(id) = target {
            present.iter().find(|(actor, _)| actor.id == id).copied()
        } else {
            explicitly_named
                .or(addressed)
                .or_else(|| addressee(&present, &words))
        }?;
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
        let standing = social.standing(member, dossier);
        let told =
            gave_clue && !player.private_clue.is_empty() && standing.attitude != Attitude::Hostile;
        let answering_private = told && private_question(&words, story);
        let released = if answering_private {
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
        let clue = if answering_private {
            Clue::Told(player.private_clue.as_str())
        } else if gave_clue && !told {
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
        if let Some(room) = &room {
            facts["now"]["room"] = room::facts(room, dossier, &withheld);
        }
        facts["npc"]["actor_id"] = json!(actor.id);
        facts["hero_visible"] = visible_hero(hero);
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
            .chain(conversation.said.iter().cloned())
            .chain(said_lines)
            .chain((!scripted.is_empty()).then(|| scripted.to_owned()))
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
        let mut turn_dossier = dossier.clone();
        if let Some(profile) = story
            .speakers
            .iter()
            .find(|profile| profile.name == actor.name)
        {
            for subject in &profile.withheld_subjects {
                if !turn_dossier.withheld_subjects.contains(subject) {
                    turn_dossier.withheld_subjects.push(*subject);
                }
            }
        }
        Some(Self {
            scene_identity: player
                .scene
                .as_ref()
                .map(|scene| format!("{}|{}", scene.title, scene.scene_asset))
                .unwrap_or_default(),
            npc_id: turn_dossier.npc_id.clone(),
            hero_id: hero.name.clone(),
            pending_quote: None,
            publication_guard: None,
            member,
            facts,
            speaker: actor.name.clone(),
            actor_id: actor.id.clone(),
            hero_name: hero.name.clone(),
            hero_index,
            typed_words: words,
            public_scene: player.narration.clone(),
            known_names: scene_names(journey, player.scene.as_ref()),
            hero_visible: visible_hero(hero),
            identity_reply: story
                .speakers
                .iter()
                .find(|profile| profile.name == actor.name)
                .and_then(|profile| profile.identity_reply.clone()),
            withheld_subjects: turn_dossier.withheld_subjects.clone(),
            private_reply: false,
            private_terms: Vec::new(),
            room,
            observed_revision: revision_key(view),
            turn: Some(Box::new(Turn {
                ledger: social.clone(),
                dossier: turn_dossier,
                scene: scene_key.to_owned(),
                standing,
                withheld,
                offered,
                said,
                conversation,
                answer_offered,
                later_lines,
            })),
        })
    }

    /// Builds the context from the asking player's own view. `None` when nobody is speaking
    /// to the party, the hero is not ready, or the typed text is empty.
    #[cfg(test)]
    pub(super) fn from_view(
        view: &rpc::ViewMessage,
        member: MemberId,
        typed: &str,
        story: &StoryFacts,
    ) -> Option<Self> {
        Self::from_view_target(view, member, typed, story, None)
    }

    fn from_view_target(
        view: &rpc::ViewMessage,
        member: MemberId,
        typed: &str,
        story: &StoryFacts,
        target: Option<&str>,
    ) -> Option<Self> {
        let Some(rpc::view_message::Audience::Player(player)) = &view.audience else {
            return None;
        };
        let journey = player.journey.as_ref()?;
        let hero = journey.own_character.as_ref()?;
        let cast = player
            .scene
            .as_ref()
            .map(|scene| scene.cast.as_slice())
            .unwrap_or_default();
        let actor = match target {
            Some(id) => Some(super::npc_target::current(cast, id)?),
            None if !cast.is_empty() => {
                let resolved = match super::npc_target::exact(typed, cast) {
                    super::npc_target::Target::Unspecified => {
                        super::npc_target::focus(cast, &journey.speaker)
                    }
                    resolved => resolved,
                };
                match resolved {
                    super::npc_target::Target::Actor(id) => {
                        Some(super::npc_target::current(cast, &id)?)
                    }
                    _ => return None,
                }
            }
            None => None,
        };
        let speaker = actor.map_or(journey.speaker.as_str(), |actor| actor.name.as_str());
        let actor_id = actor.map_or("", |actor| actor.id.as_str());
        if speaker.is_empty() || actor.is_none() && journey.dialogue.is_empty() {
            return None;
        }
        let focus = speaker == journey.speaker;
        let scripted = if focus { journey.dialogue.as_str() } else { "" };
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
            .find(|profile| profile.name == speaker);
        let mut facts = json!({
            "npc": {
                "name": speaker,
                "role": profile.map_or("the speaker the scene narration describes", |speaker| {
                    speaker.role.as_str()
                }),
                "voice": profile.map_or("", |speaker| speaker.voice.as_str()),
            },
            "scene": {
                "title": title.unwrap_or_default(),
                "narration": player.narration,
            },
            "npc_current_scripted_line": scripted,
            "characters_present": cast.iter().map(|actor| json!({"id": actor.id, "name": actor.name, "role": actor.role, "focus": actor.speaking})).collect::<Vec<_>>(),
            "your_previous_public_lines": journey.table_log.iter().rev().filter(|line| line.role == rpc::TableRole::Npc as i32 && line.kind == rpc::TableKind::Speech as i32 && (line.actor_id == actor_id && !actor_id.is_empty() || line.actor_id.is_empty() && line.speaker == speaker)).take(8).map(|line| line.text.as_str()).collect::<Vec<_>>(),
            "player": {"hero_name": hero.name, "exact_words": words},
            // Menu labels stay out of the facts: a reply that names them reads the phone's
            // menu aloud instead of answering in character.
            "offered_choices": Vec::<&str>::new(),
        });
        if !actor_id.is_empty() {
            facts["npc"]["id"] = json!(actor_id);
        }
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
        let clue_speaker =
            focus && actor.is_none_or(|actor| actor.role == rpc::SceneRole::Patron as i32);
        let told = !player.private_clue.is_empty() && clue_speaker;
        let private_terms: Vec<String> = if !player.private_clue.is_empty() {
            unheard_secret_terms(story, &public)
                .into_iter()
                .filter(|term| mentions_term(&player.private_clue, term))
                .filter(|term| {
                    !story
                        .public_names
                        .iter()
                        .any(|name| name.eq_ignore_ascii_case(term))
                })
                .collect()
        } else {
            Vec::new()
        };
        let private_reply = told && private_question(&words, story);
        if private_reply {
            facts["secret_clue_this_player_was_privately_told"] = json!(player.private_clue);
            facts["forbidden"] = json!(
                unheard_secret_terms(story, &public)
                    .into_iter()
                    .filter(|term| !private_terms.contains(term))
                    .collect::<Vec<_>>()
            );
            facts["listener_knowledge"] = json!(
                "Answer only this hero in private about the clue they were told. Do not add facts to it."
            );
        } else {
            facts["listener_knowledge"] = json!(
                "This is public table talk. Never reveal or hint at the private clue or forbidden terms."
            );
        }
        let hero_index = journey
            .party
            .iter()
            .position(|party_member| party_member.name == hero.name)
            .and_then(|position| u32::try_from(position).ok())
            .unwrap_or(0);
        Some(Self {
            scene_identity: player
                .scene
                .as_ref()
                .map(|scene| format!("{}|{}", scene.title, scene.scene_asset))
                .unwrap_or_default(),
            npc_id: String::new(),
            hero_id: hero.name.clone(),
            pending_quote: None,
            publication_guard: None,
            member,
            facts,
            speaker: speaker.to_owned(),
            actor_id: actor_id.to_owned(),
            hero_name: hero.name.clone(),
            hero_index,
            typed_words: words,
            public_scene: player.narration.clone(),
            known_names: scene_names(journey, player.scene.as_ref()),
            hero_visible: visible_hero(hero),
            identity_reply: profile.and_then(|profile| profile.identity_reply.clone()),
            withheld_subjects: profile
                .map(|profile| profile.withheld_subjects.clone())
                .unwrap_or_default(),
            private_reply,
            private_terms,
            turn: None,
            room: None,
            observed_revision: revision_key(view),
        })
    }

    /// Publishes the hero's line as public table talk, before any answer. `shown` is the display
    /// version (see `line_cleanup`); the raw words stay what the reply reasons about. A hero who
    /// was told a clue in private may type its terms, but the table does not read them.
    pub(super) fn publish_typed_line(&self, log: &TableLog, shown: &str) {
        if !self.publication_current() {
            return;
        }
        let spoken = match &self.turn {
            Some(turn) => turn.withheld.hide_released(shown),
            None => npc_dossier::redact_terms(shown, &self.private_terms),
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

fn visible_hero(hero: &rpc::CharacterSheet) -> Value {
    let visible: Vec<String> = hero
        .facts
        .iter()
        .filter(|fact| {
            [
                "sex",
                "gender",
                "presentation",
                "species",
                "ancestry",
                "build",
                "dress",
                "clothing",
                "appearance",
                "scars",
            ]
            .iter()
            .any(|label| fact.label.eq_ignore_ascii_case(label))
        })
        .map(|fact| format!("{}: {}", fact.label, fact.value))
        .collect();
    let identity = hero.identity.as_ref();
    let appearance = identity.and_then(|identity| identity.appearance.as_ref());
    let pronouns = appearance.and_then(|appearance| match appearance.pronouns.as_str() {
        "she-her" => Some(json!({"subject": "she", "object": "her", "possessive": "her", "possessive_pronoun": "hers"})),
        "he-him" => Some(json!({"subject": "he", "object": "him", "possessive": "his", "possessive_pronoun": "his"})),
        "they-them" => Some(json!({"subject": "they", "object": "them", "possessive": "their", "possessive_pronoun": "theirs"})),
        _ => None,
    });
    json!({
        "visible_facts": visible, "pronouns": pronouns,
        "appearance": appearance.map(|appearance| json!({
            "presentation": appearance.presentation, "age": appearance.age, "build": appearance.build,
            "skin": appearance.skin, "hair": appearance.hair, "hair_color": appearance.hair_color,
            "eyes": appearance.eyes, "mark": appearance.mark, "attire": appearance.attire,
        })),
        "hero_account": identity.map(|identity| json!({
            "origin": identity.origin, "occupation": identity.occupation, "motivation": identity.motivation,
            "backstory": identity.backstory, "facts": identity.facts,
            "verified_world_truth": false, "grants_mechanics": false,
        })),
    })
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

/// The server's NPC voice: asks the storyteller. No per-member reply limit (owner, 2026-10-15).
pub(super) struct NpcVoice {
    storyteller: Arc<Storyteller>,
    approach_model: ApproachModel,
}

impl NpcVoice {
    pub(super) fn hero_identity_models(&self) -> (&Storyteller, &ApproachModel) {
        (self.storyteller.as_ref(), &self.approach_model)
    }

    pub(super) async fn resolve_addressee(
        &self,
        words: &str,
        cast: &[rpc::SceneActor],
        focus: &str,
    ) -> super::npc_target::Decision {
        self.approach_model
            .resolve_addressee(words, cast, focus)
            .await
    }

    pub(super) async fn resolve_investigation_target(
        &self,
        words: &str,
        options: &[super::investigation::Target],
    ) -> super::investigation::Target {
        self.approach_model
            .resolve_investigation_target(words, options)
            .await
    }

    /// Public narration always uses the listener-safe view. Private findings are rendered
    /// separately by the granting owner, never passed to the public table log.
    pub(super) async fn describe_observation(
        &self,
        observation: &super::investigation::Observation,
    ) -> String {
        super::investigation::describe(&self.storyteller, &observation.public_view()).await
    }

    pub(super) fn new(storyteller: Arc<Storyteller>) -> Self {
        Self {
            storyteller,
            approach_model: ApproachModel::default(),
        }
    }

    pub(super) fn with_approach_model(mut self, approach_model: ApproachModel) -> Self {
        self.approach_model = approach_model;
        self
    }

    pub(super) async fn classify_input(
        &self,
        words: &str,
        public_scene: &str,
        available_things: &[String],
    ) -> super::approach_model::LineReading {
        self.approach_model
            .classify_input(words, public_scene, available_things)
            .await
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
        let exchange = self.reply_in_public_report(context, log).await?;
        (exchange.outcome != ExchangeOutcome::Unavailable).then_some(exchange.text)
    }

    async fn reply_in_public_report(
        &self,
        context: NpcReplyContext,
        log: &TableLog,
    ) -> Option<Exchange> {
        let approach = super::social::classify_keywords(&context.typed_words);
        let bounded = answer::prepare(&context, approach);
        let (text, outcome, from_model, fallback) = if let Some(answer) = bounded {
            let outcome = if answer.withheld && answer.text.is_empty() {
                ExchangeOutcome::Withheld
            } else {
                ExchangeOutcome::Spoken
            };
            (answer.text, outcome, false, None)
        } else {
            match self.model_public(&context).await {
                Ok(text) => (text, ExchangeOutcome::Spoken, true, None),
                Err(reason) => (
                    String::new(),
                    ExchangeOutcome::Unavailable,
                    false,
                    Some(reason),
                ),
            }
        };
        if !context.publication_current() {
            return None;
        }
        let private = context.private_reply && outcome == ExchangeOutcome::Spoken;
        let mut source_line_sequence = None;
        if private {
            log.push_actor(
                &context.actor_id,
                &context.speaker,
                rpc::TableRole::Npc,
                context.hero_index,
                rpc::TableKind::Action,
                &format!("murmurs to {}", context.hero_name),
            );
        } else if outcome == ExchangeOutcome::Spoken {
            source_line_sequence = log.push_actor(
                &context.actor_id,
                &context.speaker,
                rpc::TableRole::Npc,
                context.hero_index,
                rpc::TableKind::Speech,
                &text,
            );
        }
        let exchange = Exchange {
            text,
            source_line_sequence,
            outcome,
            approach,
            from_model,
            fallback,
            shift: None,
            private,
            event_proposal: None,
        };
        exchange.observe();
        Some(exchange)
    }

    /// The same publication path with a closed outcome for native diagnostics and the CLI.
    pub(super) async fn converse_report(
        &self,
        context: NpcReplyContext,
        log: &TableLog,
    ) -> Option<Exchange> {
        if context.turn.is_none() {
            self.reply_in_public_report(context, log).await
        } else {
            self.converse_in_character(context, log).await
        }
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
        Some(exchange.text)
    }

    /// [`Self::converse`] for an NPC with a backstory, reporting how the exchange went.
    pub(super) async fn converse_in_character(
        &self,
        mut context: NpcReplyContext,
        log: &TableLog,
    ) -> Option<Exchange> {
        if let Some(turn) = context.turn.as_ref()
            && turn
                .ledger
                .done_for(context.member, &turn.dossier, &turn.scene)
        {
            return None;
        }
        let dossier = &context.turn.as_ref()?.dossier;
        let question_body = room::question_body(&context);
        let reading = self
            .approach_model
            .classify_for_dossier(
                &question_body,
                &context.speaker,
                &context.public_scene,
                dossier,
            )
            .await
            .unwrap_or_else(|| {
                super::social::classify_keywords_reading_for_dossier(&context.typed_words, dossier)
            });
        let approach = reading.approach;
        let influence_request = approach.is_influence_attempt();
        context.facts["player"]["approach"] = json!(approach.label());
        context.facts["player"]["subject"] = json!(reading.subject.label());
        let mut appraised_reaction = None;
        let mut reaction_note = None;
        let mut influence_willingness = None;
        let publication_guard = context.publication_guard.clone();
        if let Some(turn) = context.turn.as_mut() {
            let similar_asks = if approach == Approach::Question {
                turn.ledger.note_question(
                    context.member,
                    &turn.dossier,
                    &turn.scene,
                    &question_body,
                )
            } else {
                0
            };
            let current = turn
                .ledger
                .emotional_state(context.member, &turn.dossier, &turn.scene);
            let visible_moment = json!({
                "hero": {"name": context.hero_name, "appearance": context.hero_visible},
                "current_room": context.facts["now"]["room"],
            });
            let appraisal = self
                .approach_model
                .appraise(
                    &turn.dossier,
                    &current,
                    turn.standing.attitude,
                    approach,
                    reading.subject,
                    &context.typed_words,
                    &visible_moment,
                    similar_asks,
                )
                .await;
            if publication_guard.as_ref().is_some_and(|guard| !guard()) {
                return None;
            }
            let (appraisal, source) = match appraisal {
                Some(appraisal) => (Some(appraisal), "jev"),
                None if approach != Approach::Intimidation => (
                    Some(turn.ledger.fallback_appraisal(
                        context.member,
                        &turn.dossier,
                        &turn.scene,
                        approach,
                        similar_asks,
                    )),
                    "fallback",
                ),
                None => (None, "fallback"),
            };
            if let Some(mut appraisal) = appraisal {
                // An ordinary low-stakes question does not rewrite a relationship.
                if matches!(approach, Approach::Question | Approach::Other)
                    && appraisal.strength != EmotionLevel::Much
                    && !matches!(
                        appraisal.reaction,
                        EmotionReaction::LosingPatience | EmotionReaction::Done
                    )
                {
                    appraisal.reaction = EmotionReaction::Unmoved;
                    appraisal.effect = super::adventure::dossier::Effect::Steady;
                }
                let previous = current.clone();
                influence_willingness =
                    influence_request.then_some(appraisal.willingness).flatten();
                let (reaction, _shift) = turn.ledger.apply_appraisal(
                    context.member,
                    &turn.dossier,
                    &turn.scene,
                    appraisal,
                );
                appraised_reaction = Some(reaction);
                turn.standing = turn.ledger.standing(context.member, &turn.dossier);
                let current =
                    turn.ledger
                        .emotional_state(context.member, &turn.dossier, &turn.scene);
                context.facts["attitude_toward_hero"] = json!(turn.standing.attitude.label());
                context.facts["emotional_state"] = json!(current);
                context.facts["npc_reaction"] = json!(format!(
                    "You feel {} toward this hero after this moment. Your current relationship is: {}.",
                    reaction.label(),
                    current.summary(),
                ));
                if current.never_forgives.is_some() {
                    influence_willingness = Some(InfluenceWillingness::Unwilling);
                }
                let summary = current.summary();
                let patience = summary
                    .split_whitespace()
                    .find_map(|part| part.strip_prefix("patience="))
                    .unwrap_or("plenty");
                super::actor::record_audit_line(
                    "gameplay.social.emotion",
                    format!(
                        "EMOTION npc={} hero={} cause={} reaction={} source={} attitude={} patience={} feelings={}",
                        turn.dossier.npc_id,
                        context.hero_name,
                        approach.label(),
                        reaction.label(),
                        source,
                        turn.standing.attitude.label(),
                        patience,
                        summary,
                    ),
                );
                if current.noticeable_change_from(&previous) {
                    reaction_note =
                        Some(current.perceived_line(&context.speaker, &context.hero_name));
                }
            }
        }
        let private_clue_withheld = context.facts.get("private_matter").is_some()
            && asks_about_private_clue(&context.typed_words);
        if private_clue_withheld {
            context.facts["npc_reaction"] = json!(
                "This hero was not told the private clue. Say plainly that you will not share what was whispered, without hinting at its contents."
            );
            context.facts["answer_from"] = json!([]);
            if let Some(facts) = context.facts.as_object_mut() {
                facts.remove("volunteer");
            }
        }
        let threat_shift = if approach == Approach::Intimidation && appraised_reaction.is_none() {
            let turn = context.turn.as_mut()?;
            let shift = turn
                .ledger
                .threaten(context.member, &turn.dossier, &turn.scene);
            turn.standing = turn.ledger.standing(context.member, &turn.dossier);
            context.facts["attitude_toward_hero"] = json!(format!("{:?}", turn.standing.attitude));
            context.facts["npc_reaction"] = json!(format!(
                "The hero has just threatened you; you are now {:?} to them.",
                turn.standing.attitude
            ));
            context.facts["answer_from"] = json!([]);
            turn.answer_offered = false;
            if let Some(facts) = context.facts.as_object_mut() {
                facts.remove("volunteer");
            }
            shift
        } else {
            None
        };
        if let Some(Shift::Level { from, to }) = threat_shift {
            reaction_note = Some(shift_line(Shift::Level { from, to }, &context.hero_name));
            let turn = context.turn.as_ref()?;
            context.facts["attitude_toward_hero"] = json!(turn.standing.attitude.label());
            eprintln!(
                "SOCIAL attitude npc=\"{}\" from={} to={} cause=threat",
                turn.dossier.npc_id,
                from.label(),
                to.label(),
            );
        }
        if approach == Approach::Intimidation && appraised_reaction.is_none() {
            let turn = context.turn.as_mut()?;
            let previous = turn
                .ledger
                .emotional_state(context.member, &turn.dossier, &turn.scene);
            let reaction =
                turn.ledger
                    .apply_legacy_threat_emotion(context.member, &turn.dossier, &turn.scene);
            appraised_reaction = Some(reaction);
            let current = turn
                .ledger
                .emotional_state(context.member, &turn.dossier, &turn.scene);
            turn.standing = turn.ledger.standing(context.member, &turn.dossier);
            context.facts["emotional_state"] = json!(current);
            context.facts["npc_reaction"] = json!(format!(
                "You feel {} toward this hero after the threat. Your current relationship is: {}.",
                reaction.label(),
                current.summary(),
            ));
            let summary = current.summary();
            let patience = summary
                .split_whitespace()
                .find_map(|part| part.strip_prefix("patience="))
                .unwrap_or("plenty");
            super::actor::record_audit_line(
                "gameplay.social.emotion",
                format!(
                    "EMOTION npc={} hero={} cause=intimidation reaction={} source=fallback attitude={} patience={} feelings={}",
                    turn.dossier.npc_id,
                    context.hero_name,
                    reaction.label(),
                    turn.standing.attitude.label(),
                    patience,
                    summary,
                ),
            );
            if current.noticeable_change_from(&previous) {
                reaction_note = Some(current.perceived_line(&context.speaker, &context.hero_name));
            }
        }
        let bounded_answer = answer::prepare(&context, approach);
        let room_answer = (approach == Approach::Question && bounded_answer.is_none())
            .then(|| room::answer(&context))
            .flatten();
        let subject_answer = if bounded_answer.is_some()
            || approach == Approach::Intimidation
            || private_clue_withheld
            || (influence_request
                && matches!(
                    influence_willingness,
                    Some(InfluenceWillingness::Unwilling | InfluenceWillingness::Hesitant)
                )) {
            None
        } else {
            let turn = context.turn.as_ref()?;
            room_answer
                .map(|answer| (answer, "room:answer".to_owned()))
                .or_else(|| {
                    npc_dossier::subject_fallback(
                        &turn.dossier,
                        reading.subject,
                        &turn.withheld,
                        &npc_dossier::Memory {
                            conversation: &turn.conversation,
                            said: &turn.said,
                            offered: &turn.offered,
                        },
                    )
                })
        };
        if let Some((answer, _)) = &subject_answer {
            context.facts["answer_from"] = json!([answer]);
            if let Some(facts) = context.facts.as_object_mut() {
                facts.remove("volunteer");
            }
        }
        if let Some(turn) = context.turn.as_mut() {
            turn.answer_offered = subject_answer
                .as_ref()
                .is_some_and(|(answer, _)| !super::speech_text::is_denial(answer));
            context.facts["already_said"] = json!(
                turn.said
                    .iter()
                    .filter(|line| !turn.withheld.leaks(line))
                    .collect::<Vec<_>>()
            );
        }
        let already_unforgiving = context.turn.as_ref().is_some_and(|turn| {
            turn.ledger
                .emotional_state(context.member, &turn.dossier, &turn.scene)
                .never_forgives
                .is_some()
        });
        let done_now = appraised_reaction == Some(EmotionReaction::Done);
        let never_forgive_now = appraised_reaction == Some(EmotionReaction::NeverForgive);
        let unwilling_now = influence_request
            && (influence_willingness == Some(InfluenceWillingness::Unwilling)
                || already_unforgiving);
        let hesitant_now =
            influence_request && influence_willingness == Some(InfluenceWillingness::Hesitant);
        if hesitant_now {
            super::actor::record_audit_line(
                "gameplay.social.influence_pending",
                format!(
                    "INFLUENCE npc={} hero={} cause=hesitant result=pending",
                    context.turn.as_ref()?.dossier.npc_id,
                    context.hero_name,
                ),
            );
        }
        super::npc_facts::trim_background_facts(&mut context.facts);
        let modeled = if done_now || never_forgive_now || unwilling_now || hesitant_now {
            Err(FallbackReason::ReplyRejected)
        } else if let Some(answer) = &bounded_answer {
            Ok((answer.text.clone(), None))
        } else {
            self.model_dialogue(&context).await
        };
        if !context.publication_current() {
            return None;
        }
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
                let (answer, answer_key): (String, Option<String>) = if done_now {
                    (
                        "I've had enough questions. Leave me be for now.".to_owned(),
                        None,
                    )
                } else if never_forgive_now {
                    ("I will never forgive you for that.".to_owned(), None)
                } else if unwilling_now {
                    ("I will not help you with that.".to_owned(), None)
                } else if hesitant_now {
                    (
                        "I need a moment before I can give you an answer.".to_owned(),
                        None,
                    )
                } else if private_clue_withheld {
                    let (answer, key) = private_clue_refusal(turn);
                    (answer, Some(key))
                } else if let Some((answer, key)) = subject_answer.clone() {
                    (answer, Some(key))
                } else {
                    npc_dossier::fallback(
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
                    )
                };
                // A prepared, confided or curt answer counts as said under its memory key.
                if let Some(key) = answer_key
                    && subject_answer.is_none()
                {
                    turn.ledger.remember_offered(
                        context.member,
                        &turn.dossier,
                        std::slice::from_ref(&key),
                    );
                }
                (answer, Some(reason))
            }
        };
        let from_model = bounded_answer.is_none() && fallback.is_none();
        let outcome = if text.is_empty()
            && bounded_answer
                .as_ref()
                .is_some_and(|answer| answer.withheld)
        {
            ExchangeOutcome::Withheld
        } else {
            ExchangeOutcome::Spoken
        };
        // What was whispered to this hero alone is answered to this hero alone: the table sees
        // that the speaker murmured, not the words.
        let private = outcome == ExchangeOutcome::Spoken
            && !turn.withheld.released().is_empty()
            && !(done_now || never_forgive_now || unwilling_now || hesitant_now);
        let event_proposal = if outcome == ExchangeOutcome::Spoken && !private {
            if let Some(room) = context.room.as_ref() {
                let result = self
                    .approach_model
                    .classify_npc_reply_event(
                        &context.actor_id,
                        &turn.dossier.npc_id,
                        &context.scene_identity,
                        &context.observed_revision,
                        &context.typed_words,
                        &text,
                        room,
                    )
                    .await;
                match result {
                    Ok(mut proposal) => {
                        if proposal.kind == super::npc_event_classifier::EventKind::IntentToGive {
                            proposal.recipient_id = room
                                .model
                                .people
                                .iter()
                                .find(|person| person.name.eq_ignore_ascii_case(&context.hero_name))
                                .map(|person| person.id.clone());
                        }
                        super::actor::record_audit_diagnostic(
                            "gameplay.npc.reply_event",
                            format!(
                                "NPC_EVENT kind={} actor={} npc={} item={} revision={} disposition=proposal",
                                proposal.kind.label(),
                                proposal.actor_id,
                                proposal.npc_id,
                                proposal.item_id.as_deref().unwrap_or("none"),
                                proposal.revision,
                            ),
                            serde_json::Map::from_iter([
                                ("kind".into(), json!(proposal.kind.label())),
                                ("actor_id".into(), json!(proposal.actor_id)),
                                ("npc_id".into(), json!(proposal.npc_id)),
                                ("item_id".into(), json!(proposal.item_id)),
                                ("source_line_id".into(), json!(proposal.source_line_id)),
                                ("revision".into(), json!(proposal.revision)),
                                ("status".into(), json!("proposal")),
                            ]),
                            &[
                                (
                                    "gameplay.npc.reply_event.kind",
                                    proposal.kind.label().into(),
                                ),
                                ("gameplay.npc.reply_event.status", "proposal".into()),
                            ],
                        );
                        Some(proposal)
                    }
                    Err(reason) => {
                        super::actor::record_audit_diagnostic(
                            "gameplay.npc.reply_event",
                            format!("NPC_EVENT status=unavailable reason={reason}"),
                            serde_json::Map::from_iter([
                                ("status".into(), json!("unavailable")),
                                ("reason".into(), json!(reason)),
                            ]),
                            &[("gameplay.npc.reply_event.status", "unavailable".into())],
                        );
                        None
                    }
                }
            } else {
                None
            }
        } else {
            None
        };
        let mut source_line_sequence = None;
        if outcome == ExchangeOutcome::Withheld {
            super::actor::record_audit_line(
                "gameplay.social.withheld",
                format!("NPC REPLY outcome=withheld npc={}", turn.dossier.npc_id),
            );
        } else if private {
            log.push_actor(
                &context.actor_id,
                &context.speaker,
                rpc::TableRole::Npc,
                context.hero_index,
                rpc::TableKind::Action,
                &format!("murmurs to {}", context.hero_name),
            );
        } else {
            source_line_sequence = log.push_actor(
                &context.actor_id,
                &context.speaker,
                rpc::TableRole::Npc,
                context.hero_index,
                rpc::TableKind::Speech,
                &text,
            );
        }
        let event_proposal = event_proposal.map(|mut proposal| {
            proposal.source_line_sequence = source_line_sequence;
            proposal
        });
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
        let mut used = used;
        if fallback.is_none()
            && let Some(answer) = &bounded_answer
        {
            used.extend(answer.keys.iter().cloned());
        }
        if let Some((_, key)) = &subject_answer {
            used.push(key.clone());
        }
        turn.ledger
            .remember_offered(context.member, &turn.dossier, &used);
        let reaction_note = reaction_note.or_else(|| match recorded.shift {
            Some(shift @ Shift::Level { .. }) => Some(shift_line(shift, &context.hero_name)),
            _ => None,
        });
        if outcome == ExchangeOutcome::Spoken
            && let Some(note) = reaction_note
        {
            log.push_actor(
                &context.actor_id,
                &context.speaker,
                rpc::TableRole::Npc,
                context.hero_index,
                rpc::TableKind::Action,
                &note,
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
        let exchange = Exchange {
            text,
            source_line_sequence,
            outcome,
            approach,
            from_model,
            fallback,
            shift: recorded.shift,
            private,
            event_proposal,
        };
        exchange.observe();
        Some(exchange)
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
                StorytellerError::Rejected { failure, words } => {
                    let detail = words
                        .as_deref()
                        .map(|words| turn.withheld.redacted_detail(&turn.dossier, words))
                        .unwrap_or_else(|| "<no reply words>".to_owned());
                    eprintln!(
                        "{}",
                        rejected_reply_log_line(&turn.dossier.npc_id, &failure, &detail)
                    );
                    FallbackReason::ReplyRejected
                }
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

    #[cfg(test)]
    pub(super) async fn reply(&self, context: NpcReplyContext) -> Option<String> {
        self.model_public(&context).await.ok()
    }

    async fn model_public(&self, context: &NpcReplyContext) -> Result<String, FallbackReason> {
        let rating = content_rating();
        if !self.storyteller.is_on() {
            return Err(FallbackReason::ModelOff);
        }
        let forbidden = context.facts["forbidden"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        let slots = [Slot::new("reply", REPLY_CHARACTERS).withholding(forbidden)];
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
            .map_err(|error| match error {
                StorytellerError::Off => FallbackReason::ModelOff,
                StorytellerError::BudgetExhausted => FallbackReason::BudgetExhausted,
                StorytellerError::Rejected { .. } => FallbackReason::ReplyRejected,
                StorytellerError::InvalidRequest(label) => FallbackReason::RequestInvalid(label),
                StorytellerError::DeadlineExceeded | StorytellerError::Provider(_) => {
                    FallbackReason::CallFailed
                }
            })?;
        let reply = values
            .get("reply")
            .map(|text| strip_outer_quotes(text))
            .filter(|text| !text.is_empty())
            .ok_or(FallbackReason::EmptyReply)?;
        // A recipient grant never authorizes those terms in a public provider reply.
        if !context.private_reply
            && npc_dossier::redact_terms(&reply, &context.private_terms) != reply
        {
            return Err(FallbackReason::ReplyRejected);
        }
        Ok(reply)
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

fn rejected_reply_log_line(npc_id: &str, failure: &ValidationFailure, detail: &str) -> String {
    let detail = serde_json::to_string(detail).unwrap_or_else(|_| "\"<unavailable>\"".to_owned());
    format!(
        "NPC REPLY rejected npc={npc_id} rule={} detail={detail}",
        failure.label()
    )
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
#[path = "npc_room_tests.rs"]
mod room_tests;

#[cfg(test)]
mod tests {
    use super::super::adventure::dossier::Attitude;
    use super::*;
    use std::time::Duration;

    const CLUE: &str = "The courier quietly tells you: the sealed packet is addressed to Vell at the Harbor Inn. Keep its destination between you.";

    #[test]
    fn authored_background_actor_uses_its_own_persona_and_canonical_identity() {
        assert!(
            STYLE.chars().count() <= 600,
            "the shared storyteller admits this style"
        );
        let mut told = view(CLUE, "Courier");
        let Some(rpc::view_message::Audience::Player(player)) = &mut told.audience else {
            panic!()
        };
        player.scene.as_mut().unwrap().cast = vec![
            rpc::SceneActor {
                id: "courier".into(),
                name: "Courier".into(),
                speaking: true,
                ..Default::default()
            },
            rpc::SceneActor {
                id: "bandit".into(),
                name: "Bandit".into(),
                ..Default::default()
            },
        ];
        player
            .journey
            .as_mut()
            .unwrap()
            .table_log
            .push(rpc::TableLine {
                sequence: 1,
                actor_id: "bandit".into(),
                speaker: "Bandit".into(),
                role: rpc::TableRole::Npc as i32,
                kind: rpc::TableKind::Speech as i32,
                text: "Give me that packet.".into(),
                ..Default::default()
            });
        let context = NpcReplyContext::from_view_target(
            &told,
            member(1),
            "Bandit, who sent you?",
            &listed_story(),
            Some("bandit"),
        )
        .unwrap();
        assert_eq!(context.actor_id, "bandit");
        assert_eq!(context.speaker, "Bandit");
        assert_eq!(context.facts["npc"]["id"], "bandit");
        assert_eq!(context.facts["npc"]["voice"], "sneering and rough");
        assert_eq!(context.facts["npc_current_scripted_line"], "");
        assert!(
            context
                .facts
                .get("secret_clue_this_player_was_privately_told")
                .is_none()
        );
        assert_eq!(
            context.facts["your_previous_public_lines"][0],
            "Give me that packet."
        );
        let courier = NpcReplyContext::from_view_target(
            &told,
            member(1),
            "Courier, who sent you?",
            &listed_story(),
            Some("courier"),
        )
        .unwrap();
        assert_eq!(courier.actor_id, "courier");
        assert_eq!(courier.facts["npc"]["voice"], "hurried and anxious");
        assert_eq!(courier.facts["your_previous_public_lines"], json!([]));
    }
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
            scenes: Vec::new(),
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
            scenes: Vec::new(),
        }
    }

    #[test]
    fn rejected_reply_diagnostic_names_its_rule_and_words_srd_5_2_1_page_184() {
        // SRD 5.2.1, p. 184: this records dialogue validation, not an Influence check.
        let line = rejected_reply_log_line(
            "patron:ferry-clerk",
            &ValidationFailure::RepeatedSentence {
                slot: "reply".to_owned(),
            },
            "Please warn Tomas before dawn.",
        );
        assert_eq!(
            line,
            "NPC REPLY rejected npc=patron:ferry-clerk rule=repeated_sentence detail=\"Please warn Tomas before dawn.\""
        );
    }

    #[test]
    fn rejected_reply_diagnostic_redacts_secret_terms_to_their_key_srd_5_2_1_page_184() {
        // SRD 5.2.1, p. 184: rejection diagnostics identify the dialogue rule without exposing secrets.
        let adventure = super::super::npc_dossier_tests::adventure(0x7f4a_7c15);
        let dossier = adventure
            .dossiers
            .iter()
            .find(|dossier| dossier.role == NpcRole::Patron)
            .unwrap();
        let withheld = npc_dossier::Withheld::new(
            dossier,
            vec!["Vell".to_owned()],
            Vec::new(),
            content_rating(),
        );
        let detail = withheld.redacted_detail(dossier, "The packet goes to Vell.");
        let line = rejected_reply_log_line(
            &dossier.npc_id,
            &ValidationFailure::ForbiddenTerm {
                slot: "reply".to_owned(),
            },
            &detail,
        );

        assert!(!line.contains("Vell"), "{line}");
        assert!(line.contains("rule=forbidden_term"), "{line}");
        assert!(line.contains("adventure_secret."), "{line}");
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
                    identity_reply: None,
                    withheld_subjects: Vec::new(),
                },
                SpeakerProfile {
                    name: "Bandit".to_owned(),
                    gender: None,
                    age: None,
                    role: "a hostile bandit fighting the heroes".to_owned(),
                    voice: "sneering and rough".to_owned(),
                    identity_reply: None,
                    withheld_subjects: Vec::new(),
                },
            ],
            scene_lines: Vec::new(),
            scenes: Vec::new(),
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

        // An explicitly private follow-up uses the actual recipient grant.
        assert_eq!(context.facts["forbidden"], json!([]));
        assert!(context.private_reply);
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
                .contains("public table talk")
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
                .contains("only this hero in private")
        );
        assert_eq!(context.facts["forbidden"], json!([]));
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

    fn harbor_question(target: &str, words: &str, told: bool) -> NpcReplyContext {
        let pack: PackFile =
            serde_json::from_str(include_str!("../../../../content/harbor-journey/pack.json"))
                .unwrap();
        harbor_question_from_pack(&pack, target, words, told)
    }

    fn harbor_question_from_pack(
        pack: &PackFile,
        target: &str,
        words: &str,
        told: bool,
    ) -> NpcReplyContext {
        let scene = pack
            .scenes
            .iter()
            .find(|scene| scene.key == SceneKey::DialogueSeal)
            .unwrap();
        let mut projected = view(if told { &pack.courier_clue } else { "" }, &scene.speaker);
        let Some(rpc::view_message::Audience::Player(player)) = projected.audience.as_mut() else {
            panic!("player")
        };
        player.narration = scene.narration.clone();
        player.scene = Some(rpc::GameplayScene {
            title: scene.title.clone(),
            cast: super::super::wire::scene_cast(scene),
            ..Default::default()
        });
        player.journey.as_mut().unwrap().dialogue = scene.dialogue.clone();
        let context = NpcReplyContext::for_target(
            &projected,
            member(1),
            words,
            &StoryFacts::from_pack(pack),
            &SocialLedger::none(),
            "DialogueSeal",
            Some(target),
        )
        .unwrap();
        assert!(
            context.turn.is_none(),
            "the shipped harbor has no generated dossiers"
        );
        context
    }

    #[tokio::test]
    async fn harbor_without_dossiers_never_invents_the_observed_personal_claims() {
        let endpoint = FakeEndpoint::start(vec![Reply::Output(
            json!({"reply": "I SAW it. I wasn’t hurt. Nobody sent me."}),
        )]);
        let (voice, cache) = voice(&endpoint, "harbor-personal-authority");
        for words in [
            "Are you hurt?",
            "Have you made this delivery before?",
            "What payment was promised?",
            "Were you paid?",
            "How do you know the seal was broken before port?",
        ] {
            let context = harbor_question("courier", words, false);
            let exchange = voice
                .converse_report(context, &TableLog::new())
                .await
                .unwrap();
            assert_eq!(exchange.outcome, ExchangeOutcome::Spoken);
            assert_eq!(exchange.text, "I can't give you a clear answer to that.");
            assert_eq!(exchange.fallback, None);
            assert!(!exchange.from_model);
        }
        let context = harbor_question("courier", "Who are you and what happened?", false);
        let scripted = context.facts["npc_current_scripted_line"]
            .as_str()
            .unwrap()
            .to_owned();
        let text = voice.converse(context, &TableLog::new()).await.unwrap();
        assert_eq!(
            text,
            format!("You can call me the courier. My name can wait. {scripted}")
        );
        assert_eq!(endpoint.connections(), 0);
        if let Err(error) = std::fs::remove_dir_all(cache) {
            assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        }
    }

    #[tokio::test]
    async fn harbor_identity_uses_authored_public_speech_instead_of_the_scene_label() {
        let source: PackFile =
            serde_json::from_str(include_str!("../../../../content/harbor-journey/pack.json"))
                .unwrap();
        for (identity_reply, expected) in [
            (
                source.speakers[0].identity_reply.clone(),
                "You can call me the courier. My name can wait.",
            ),
            (Some("I'm Mara.".to_owned()), "I'm Mara."),
            (None, "I can't give you a clear answer to that."),
            (
                Some("My name is Vell.".to_owned()),
                "I can't give you a clear answer to that.",
            ),
        ] {
            let mut pack = source.clone();
            pack.speakers[0].identity_reply = identity_reply;
            let context =
                harbor_question_from_pack(&pack, "courier", "courier, who are you?", true);
            assert_eq!(context.actor_id, "courier");
            assert_eq!(context.speaker, "Courier");
            let log = TableLog::new();
            let exchange = NpcVoice::silent()
                .converse_report(context, &log)
                .await
                .unwrap();
            assert_eq!(exchange.outcome, ExchangeOutcome::Spoken);
            assert_eq!(exchange.text, expected);
            assert!(!exchange.from_model);
            assert_eq!(exchange.fallback, None);
            assert!(log.lines().iter().any(|line| {
                line.role == rpc::TableRole::Npc as i32
                    && line.actor_id == "courier"
                    && line.speaker == "Courier"
                    && line.text == expected
            }));
        }
    }

    #[tokio::test]
    async fn harbor_guarded_identity_closes_pending_and_preserves_other_authorized_clauses() {
        let mut pack: PackFile =
            serde_json::from_str(include_str!("../../../../content/harbor-journey/pack.json"))
                .unwrap();
        pack.speakers[0].identity_reply = Some("I'm Mara.".to_owned());
        pack.speakers[0]
            .withheld_subjects
            .push(QuestionSubject::Identity);
        let log = TableLog::new();
        let context = harbor_question_from_pack(&pack, "courier", "courier, who are you?", false);
        let exchange = {
            let _pending = log.reply_pending(&context.speaker);
            NpcVoice::silent()
                .converse_report(context, &log)
                .await
                .unwrap()
        };
        assert_eq!(exchange.outcome, ExchangeOutcome::Withheld);
        assert!(exchange.text.is_empty() && !exchange.from_model);
        assert_eq!(exchange.fallback, None);
        assert!(
            log.lines().is_empty(),
            "silence has no NPC caption or TTS source"
        );
        let mut shown = view("", "Courier");
        log.attach(&mut shown);
        let Some(rpc::view_message::Audience::Player(player)) = shown.audience else {
            panic!("player")
        };
        assert!(player.journey.unwrap().reply_speaker.is_empty());
        let context =
            harbor_question_from_pack(&pack, "courier", "Who are you and what happened?", false);
        let scripted = context.facts["npc_current_scripted_line"]
            .as_str()
            .unwrap()
            .to_owned();
        let compound = NpcVoice::silent()
            .converse_report(context, &log)
            .await
            .unwrap();
        assert_eq!(compound.outcome, ExchangeOutcome::Spoken);
        assert_eq!(compound.text, scripted);
        assert!(!compound.text.contains("Mara"));
    }

    #[tokio::test]
    async fn harbor_withholding_uses_the_normal_wrapper_and_closes_pending_without_tts_source() {
        let endpoint = FakeEndpoint::start(vec![Reply::Status(500, json!({"error": {}}))]);
        let (voice, cache) = voice(&endpoint, "harbor-withholding");
        let log = TableLog::new();
        let wake = log.subscribe();
        for words in [
            "Why do you want the packet?",
            "Who sent you?",
            "Bandit, why are you following the Courier?",
            "Why he's following the courier?",
        ] {
            let context = harbor_question("bandit", words, false);
            let reply = {
                let _pending = log.reply_pending(&context.speaker);
                voice.converse(context, &log).await
            };
            assert_eq!(reply.as_deref(), Some(""));
            let exchange = voice
                .converse_report(harbor_question("bandit", words, false), &log)
                .await
                .unwrap();
            assert_eq!(exchange.outcome, ExchangeOutcome::Withheld);
            assert_eq!(exchange.fallback, None);
            assert!(!exchange.from_model && exchange.text.is_empty());
        }
        let mut shown = view("", "Courier");
        log.attach(&mut shown);
        let Some(rpc::view_message::Audience::Player(player)) = shown.audience else {
            panic!("player")
        };
        assert!(player.journey.unwrap().reply_speaker.is_empty());
        assert!(wake.has_changed().unwrap());
        assert!(
            log.lines().is_empty(),
            "no NPC caption or voice line exists for silence"
        );
        assert_eq!(endpoint.connections(), 0);
        let off = NpcVoice::silent()
            .converse_report(harbor_question("courier", "Hello?", false), &log)
            .await
            .unwrap();
        assert_eq!(off.outcome, ExchangeOutcome::Unavailable);
        assert_eq!(off.fallback, Some(FallbackReason::ModelOff));
        let failed = voice
            .converse_report(harbor_question("courier", "Hello?", false), &log)
            .await
            .unwrap();
        assert_eq!(failed.outcome, ExchangeOutcome::Unavailable);
        assert_eq!(failed.fallback, Some(FallbackReason::CallFailed));
        let mut stale = harbor_question("bandit", "Who sent you?", false);
        stale.publication_guard = Some(Arc::new(|| false));
        assert!(voice.converse_report(stale, &log).await.is_none());
        assert!(log.lines().is_empty());
        if let Err(error) = std::fs::remove_dir_all(cache) {
            assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        }
    }

    #[tokio::test]
    async fn harbor_empty_provider_text_is_unavailable_and_never_intentional_silence() {
        let endpoint = FakeEndpoint::start(vec![Reply::Output(json!({"reply": "\"\""}))]);
        let (voice, cache) = voice(&endpoint, "harbor-empty-outcome");
        let log = TableLog::new();
        let exchange = voice
            .converse_report(harbor_question("courier", "Hello?", false), &log)
            .await
            .unwrap();
        assert_eq!(exchange.outcome, ExchangeOutcome::Unavailable);
        assert_eq!(exchange.fallback, Some(FallbackReason::EmptyReply));
        assert!(log.lines().is_empty());
        std::fs::remove_dir_all(cache).unwrap();
    }

    #[tokio::test]
    async fn harbor_public_provider_reply_cannot_echo_a_recipients_private_destination() {
        let leak = "The packet goes to Vell at the Harbor Inn.";
        let endpoint = FakeEndpoint::start(vec![
            Reply::Output(json!({"reply": leak})),
            Reply::Output(json!({"reply": leak})),
            Reply::Output(json!({"reply": "Mind the rain."})),
        ]);
        let (voice, cache) = voice(&endpoint, "harbor-public-provider-leak");
        let context = harbor_question("courier", "Hello?", true);
        assert!(context.turn.is_none() && !context.private_reply);
        assert!(
            context
                .private_terms
                .iter()
                .any(|term| term == "Harbor Inn")
        );
        assert!(
            context
                .facts
                .get("secret_clue_this_player_was_privately_told")
                .is_none()
        );
        let log = TableLog::new();
        let rejected = voice.converse_report(context, &log).await.unwrap();
        assert_eq!(rejected.outcome, ExchangeOutcome::Unavailable);
        assert_eq!(rejected.fallback, Some(FallbackReason::ReplyRejected));
        assert!(rejected.text.is_empty() && !rejected.private);
        assert!(
            log.lines().is_empty(),
            "a provider echo never becomes a public caption or TTS source"
        );
        assert_eq!(
            endpoint.connections(),
            2,
            "only the established bounded repair attempt runs"
        );
        let public = voice
            .converse_report(harbor_question("courier", "Hello?", true), &log)
            .await
            .unwrap();
        assert_eq!(public.outcome, ExchangeOutcome::Spoken);
        assert_eq!(public.text, "Mind the rain.");
        assert_eq!(public.fallback, None);
        assert!(
            log.lines()
                .iter()
                .all(|line| !line.text.contains("Vell") && !line.text.contains("Harbor Inn"))
        );
        std::fs::remove_dir_all(cache).unwrap();
    }

    #[tokio::test]
    async fn harbor_private_followup_and_next_public_reply_keep_their_own_audiences() {
        let endpoint = FakeEndpoint::start(vec![Reply::Output(
            json!({"reply": "The packet is addressed to Vell at the Harbor Inn."}),
        )]);
        let (voice, cache) = voice(&endpoint, "harbor-private-scope");
        let log = TableLog::new();
        let context = harbor_question("courier", "Where is the packet going?", true);
        assert!(context.private_reply);
        let private = voice.converse_report(context, &log).await.unwrap();
        assert!(private.private);
        assert!(private.text.contains("Vell"));
        let public_context = harbor_question("courier", "Who are you?", true);
        assert!(!public_context.private_reply);
        assert!(
            public_context
                .facts
                .get("secret_clue_this_player_was_privately_told")
                .is_none()
        );
        let public = voice.converse_report(public_context, &log).await.unwrap();
        assert!(!public.private);
        assert_eq!(
            public.text,
            "You can call me the courier. My name can wait."
        );
        assert!(
            log.lines()
                .iter()
                .any(|line| line.kind == rpc::TableKind::Speech as i32 && line.text == public.text)
        );
        assert!(
            log.lines()
                .iter()
                .all(|line| !line.text.contains("Vell") && !line.text.contains("Harbor Inn"))
        );
        std::fs::remove_dir_all(cache).unwrap();
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

    #[tokio::test(flavor = "current_thread")]
    async fn reply_completion_revalidates_its_publication_scope_before_the_table_log() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let current = Arc::new(AtomicBool::new(true));
        let changed = current.clone();
        let endpoint = FakeEndpoint::start(vec![Reply::Compute(Arc::new(move |_| {
            changed.store(false, Ordering::SeqCst);
            json!({"reply":"Keep your voice low, friend."})
        }))]);
        let (voice, cache) = voice(&endpoint, "routing-stale-publication");
        let mut context = question("Who is following you?");
        context.publication_guard = Some(Arc::new(move || current.load(Ordering::SeqCst)));
        let log = TableLog::new();
        assert!(voice.reply_in_public(context, &log).await.is_none());
        assert!(
            log.lines().is_empty(),
            "a model completion from a departed actor is never published"
        );
        assert_eq!(endpoint.connections(), 1);
        std::fs::remove_dir_all(cache).unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn authored_target_identity_survives_model_reply_public_log_and_voice_projection() {
        let endpoint = FakeEndpoint::start(vec![
            Reply::Output(json!({"reply":"Give me that packet."})),
            Reply::Output(json!({"reply":"Keep it safe, friend."})),
        ]);
        let (voice, cache) = voice(&endpoint, "routing-distinct-public-actors");
        let mut told = view("", "Courier");
        let Some(rpc::view_message::Audience::Player(player)) = &mut told.audience else {
            panic!()
        };
        let scene = player.scene.as_mut().unwrap();
        scene.cast = vec![
            rpc::SceneActor {
                id: "courier".into(),
                name: "Courier".into(),
                speaking: true,
                ..Default::default()
            },
            rpc::SceneActor {
                id: "bandit".into(),
                name: "Bandit".into(),
                ..Default::default()
            },
        ];
        let cast = scene.clone();
        let log = TableLog::new();
        for (words, from_model) in [("who sent you?", false), ("Speak to me.", true)] {
            for (id, name) in [("bandit", "Bandit"), ("courier", "Courier")] {
                let context = NpcReplyContext::from_view_target(
                    &told,
                    member(1),
                    &format!("{name}, {words}"),
                    &listed_story(),
                    Some(id),
                )
                .unwrap();
                assert!(voice.converse(context, &log).await.is_some());
                let line = log.lines().last().unwrap().clone();
                assert_eq!(line.actor_id, id);
                assert_eq!(line.speaker, name);
                if !from_model {
                    assert_eq!(line.text, "I can't give you a clear answer to that.");
                }
                let display = rpc::DisplayGameplayView {
                    scene: Some(cast.clone()),
                    journey: Some(rpc::JourneyView {
                        table_log: vec![line],
                        ..Default::default()
                    }),
                    ..Default::default()
                };
                let voices = crate::voice_line::display_lines(&display);
                assert_eq!(voices.last().unwrap().actor_id(), Some(id));
                assert_eq!(voices.last().unwrap().speaker(), Some(name));
            }
            assert_eq!(endpoint.connections(), if from_model { 2 } else { 0 });
        }
        assert_eq!(endpoint.connections(), 2);
        let bodies = endpoint.bodies();
        assert!(
            bodies[0]["input"]
                .as_str()
                .unwrap()
                .contains("sneering and rough")
        );
        assert!(
            bodies[1]["input"]
                .as_str()
                .unwrap()
                .contains("hurried and anxious")
        );
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
        let first = holder_question("What can you say aloud?");
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
                    "What can you say aloud?"
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
                    rpc::TableKind::Action as i32,
                    "murmurs to Brannoch"
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
        assert!(sent.contains("public table talk"));
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
    async fn a_seventh_reply_within_a_minute_is_answered_like_the_first() {
        let endpoint = FakeEndpoint::start(vec![Reply::Output(json!({"reply": "Quiet now."}))]);
        let (voice, cache) = voice(&endpoint, "voice-rate");

        for _ in 0..6 {
            assert_eq!(
                voice.reply(question("Hello?")).await.as_deref(),
                Some("Quiet now.")
            );
        }
        // No per-member reply limit (owner, 2026-10-15): the seventh is answered too.
        let seventh = voice.reply(question("Hello?")).await;

        assert_eq!(seventh.as_deref(), Some("Quiet now."));
        assert_eq!(
            endpoint.connections(),
            1,
            "the repeats were served from the cache"
        );
        std::fs::remove_dir_all(cache).unwrap();
    }
}
