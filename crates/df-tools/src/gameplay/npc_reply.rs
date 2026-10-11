//! The scene's speaker answers, in character, what a player types to them.
//!
//! The reply is prose only. It is produced after the native actor has classified the typed
//! line, off the actor thread, and is delivered solely in that player's own dialogue
//! response. A typed line never executes an action or a check; the reply cannot either,
//! because the storyteller rejects game numbers that are not in the facts.
//!
//! Listener safety: the facts hold only what the asking player has already been told. The
//! scene's secret clue is included only when this member has privately received it;
//! otherwise its key terms go into `forbidden`, and the storyteller refuses any reply that
//! uses them.

use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};

use df_protocol::common as rpc;
use df_types::MemberId;
use serde_json::{Value, json};

use super::storyteller::{Expression, Slot, Storyteller, contains_whole_word};

/// Longest in-character reply, in characters.
pub(super) const REPLY_CHARACTERS: usize = 220;
/// Only this much of the player's words is shown to the model.
const MAXIMUM_PLAYER_CHARACTERS: usize = 500;
const REPLIES_PER_WINDOW: usize = 6;
const WINDOW: Duration = Duration::from_secs(60);

const STYLE: &str = "A spoken line in character, in the first person, with no quotation marks. \
It may refuse or deflect. It never promises mechanical results. When asked what to do, it \
nudges toward the offered choices. Plain, brief and warm.";

/// Everything one reply needs, captured by the actor and owned so the model call can run
/// elsewhere.
pub(super) struct NpcReplyContext {
    member: MemberId,
    facts: Value,
}

impl NpcReplyContext {
    /// Builds the context from the asking player's own view. `None` when nobody is speaking
    /// to the party, the hero is not ready, or the typed text is empty.
    pub(super) fn from_view(
        view: &rpc::ViewMessage,
        member: MemberId,
        typed: &str,
        secret_clue: &str,
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
        let mut facts = json!({
            "npc": {
                "name": journey.speaker,
                "role": "the speaker the scene narration describes",
            },
            "scene": {
                "title": title.unwrap_or_default(),
                "narration": player.narration,
            },
            "npc_current_scripted_line": journey.dialogue,
            "player": {"hero_name": hero.name, "exact_words": words},
            "offered_choices": choices,
        });
        if player.private_clue.is_empty() {
            let public = format!(
                "{} {} {} {} {}",
                title.unwrap_or_default(),
                player.narration,
                journey.dialogue,
                hero.name,
                choices.join(" "),
            );
            facts["forbidden"] = json!(secret_terms(secret_clue, &public));
        } else {
            facts["secret_clue_this_player_was_privately_told_and_may_be_reminded_of"] =
                json!(player.private_clue);
            facts["forbidden"] = json!([]);
        }
        Some(Self { member, facts })
    }
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
}

impl NpcVoice {
    pub(super) fn new(storyteller: Arc<Storyteller>) -> Self {
        Self {
            storyteller,
            limiter: Arc::default(),
        }
    }

    /// A voice that never answers.
    #[cfg(test)]
    pub(super) fn silent() -> Self {
        Self::new(Arc::new(Storyteller::disabled()))
    }

    /// The reply text, or `None` when the storyteller is off, the member is over its bounds,
    /// or the model failed, timed out or broke a rule. `None` means "answer as before".
    pub(super) async fn reply(&self, context: NpcReplyContext) -> Option<String> {
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
            })
            .await
            .ok()?;
        let reply = strip_outer_quotes(values.get("reply")?);
        (!reply.is_empty()).then_some(reply)
    }
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
    use super::*;

    const CLUE: &str = "The courier quietly tells you: the sealed packet is addressed to Vell at the Harbor Inn. Keep its destination between you.";

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
    fn a_player_without_the_clue_gets_its_key_terms_forbidden_and_not_its_text() {
        let context =
            NpcReplyContext::from_view(&view("", "Courier"), member(1), "Where is it going?", CLUE)
                .unwrap();

        let facts = &context.facts;
        assert_eq!(facts["player"]["exact_words"], "Where is it going?");
        assert_eq!(facts["player"]["hero_name"], "Brannoch");
        assert_eq!(facts["npc"]["name"], "Courier");
        assert_eq!(
            facts["npc_current_scripted_line"],
            "Please. I cannot do this alone."
        );
        assert_eq!(
            facts["offered_choices"],
            json!(["Ask the courier what he knows"])
        );
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

        let context = NpcReplyContext::from_view(&shared, member(1), "Who?", CLUE).unwrap();

        assert_eq!(context.facts["forbidden"], json!(["Harbor Inn", "Inn"]));
    }

    #[test]
    fn a_player_who_was_told_the_clue_may_hear_it_again() {
        let told = view(
            "The packet is addressed to Vell at the Harbor Inn.",
            "Courier",
        );

        let context = NpcReplyContext::from_view(&told, member(1), "Where exactly?", CLUE).unwrap();

        assert_eq!(context.facts["forbidden"], json!([]));
        assert_eq!(
            context.facts["secret_clue_this_player_was_privately_told_and_may_be_reminded_of"],
            "The packet is addressed to Vell at the Harbor Inn."
        );
    }

    #[test]
    fn nobody_answers_when_no_one_is_speaking_or_the_text_is_empty() {
        assert!(NpcReplyContext::from_view(&view("", ""), member(1), "Hello", CLUE).is_none());
        assert!(
            NpcReplyContext::from_view(&view("", "Courier"), member(1), " \n ", CLUE).is_none()
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
        NpcReplyContext::from_view(&view("", "Courier"), member(1), words, CLUE).unwrap()
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
