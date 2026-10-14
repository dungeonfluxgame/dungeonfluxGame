//! Raw dialogue remains inert; only a separate native confirmed choice reaches Session.
use std::collections::BTreeMap;
use std::mem::size_of;

use df_intent::dialogue::{DialogueLimits, classify_dialogue, confirm_final_input};
use df_model::intent::{DiscourseContext, InputFinality, IntentDisposition, RawInputRef};
use df_persistence::local_demo_scope::{LocalDemoRole, PLAYER};
use df_protocol::common as rpc;
use df_types::{MemberId, OperationId};
use prost::Message;
use sha2::{Digest, Sha256};
use tokio::sync::oneshot;
use tonic::{Request, Response, Status};

use super::{
    Service, actor, credential, journey, model,
    npc_reply::{NpcReplyContext, StoryFacts},
    wire,
};

const MAXIMUM_MESSAGE_BYTES: usize = 8192;
const MAXIMUM_TEXT_BYTES: usize = 4096;
const NOTE_COMMAND_OVERHEAD_BYTES: usize = "write down ".len();
const MAXIMUM_PENDING_MEMBERS: usize = 32;
const MAXIMUM_PENDING_BYTES: usize = 64 * 1024 * 1024;

pub(super) enum DialogueRequest {
    Raw(Vec<u8>),
    Confirm(Vec<u8>),
    Cancel(Vec<u8>),
}

#[cfg(test)]
mod notebook_command_tests {
    use super::{DiscourseContext, NotebookCommand, notebook_command};

    // SRD 5.2.1 p. N/A: this test covers the notebook command, not a game rule.
    #[test]
    fn about_the_game_note_prefixes_are_saved_without_parsing_other_contexts() {
        assert!(matches!(
            notebook_command(DiscourseContext::Meta, "note: Tomas is a mapmaker."),
            Some(NotebookCommand::Note("Tomas is a mapmaker."))
        ));
        assert!(matches!(
            notebook_command(
                DiscourseContext::Meta,
                " Write Down Kelbin wants the letter. "
            ),
            Some(NotebookCommand::Note("Kelbin wants the letter."))
        ));
        assert!(notebook_command(DiscourseContext::Question, "note: no").is_none());
        assert!(
            notebook_command(DiscourseContext::Meta, "we should write down the clues").is_none()
        );
    }

    // SRD 5.2.1 p. N/A: this test covers the notebook command, not a game rule.
    #[test]
    fn party_share_command_requires_a_bounded_order() {
        assert!(matches!(
            notebook_command(DiscourseContext::Meta, "share clue 42"),
            Some(NotebookCommand::Share(42))
        ));
        assert!(notebook_command(DiscourseContext::Meta, "share clue many").is_none());
    }

    // SRD 5.2.1 p. N/A: this test checks the product text bound, not a game rule.
    #[test]
    fn party_note_command_accepts_a_full_spoken_line() {
        let text = format!("note: {}", "x".repeat(super::MAXIMUM_TEXT_BYTES));
        assert_eq!(
            super::dialogue_text_limit(DiscourseContext::Meta, &text),
            text.len()
        );
        assert_eq!(
            super::dialogue_text_limit(DiscourseContext::Question, &text),
            super::MAXIMUM_TEXT_BYTES
        );
        assert_eq!(
            super::maximum_request_text_bytes(DiscourseContext::Meta),
            super::MAXIMUM_TEXT_BYTES + super::NOTE_COMMAND_OVERHEAD_BYTES
        );
    }
}

#[cfg(test)]
mod observation_label_tests {
    #[test]
    fn observation_target_is_a_valid_srd_5_2_1_search_label_page_187() {
        // SRD 5.2.1 p. 187: Search is a Wisdom check to discern something that isn't obvious.
        assert_eq!(
            super::observation_thing_label("bell chamber"),
            Some("bell-chamber".to_owned())
        );
        assert_eq!(
            super::observation_thing_label("Ulric's face"),
            Some("Ulric-s-face".to_owned())
        );
        assert_eq!(super::observation_thing_label(" ! "), None);
        assert!(
            df_types::RevisionLabel::new(super::observation_thing_label("bell chamber").as_deref())
                .is_ok()
        );
    }
}

impl DialogueRequest {
    pub(super) fn retained_heap_bytes(&self) -> usize {
        match self {
            Self::Raw(bytes) | Self::Confirm(bytes) | Self::Cancel(bytes) => bytes.capacity(),
        }
    }
}

pub(super) enum DialogueReply {
    /// The response, plus what a spoken in-character reply needs when the line qualifies.
    Raw(rpc::DialogueResponse, Option<NpcReplyContext>),
    Confirmed(rpc::SubmitActionResponse),
    Cancelled(rpc::DialogueResponse),
}

struct PendingConfirmation {
    input: RawInputRef,
    token: [u8; 32],
    offer: String,
    kind: rpc::GameplayActionKind,
    selection: Option<([u8; 32], OperationId)>,
    charge: usize,
}

/// Owned by the single native Actor. Raw text is never retained here.
#[derive(Default)]
pub(super) struct DialogueState {
    pending: BTreeMap<MemberId, PendingConfirmation>,
}

fn encode<M: Message>(request: &M) -> Result<Vec<u8>, Status> {
    let length = request.encoded_len();
    if length > MAXIMUM_MESSAGE_BYTES {
        return Err(Status::resource_exhausted(
            "dialogue message bound exceeded",
        ));
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| Status::resource_exhausted("dialogue admission capacity unavailable"))?;
    request
        .encode(&mut bytes)
        .map_err(|_| Status::invalid_argument("dialogue encoding invalid"))?;
    Ok(bytes)
}

fn observation_thing_label(thing: &str) -> Option<String> {
    let label = thing
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .take(16)
        .collect::<Vec<_>>()
        .join("-");
    (!label.is_empty()).then_some(label)
}

fn is_observation_submission(
    request: &rpc::RawDialogueRequest,
    reading: &super::approach_model::LineReading,
) -> Result<bool, Status> {
    Ok(finality(request.finality)? == InputFinality::Final && reading.intent.is_attempt())
}

#[cfg(test)]
mod classified_line_tests {
    use super::*;

    #[test]
    fn plan_context_does_not_suppress_a_final_search_attempt_srd_521_page_187() {
        // SRD 5.2.1 p. 187: the Search action resolves with a Wisdom check.
        let reading = super::super::approach_model::LineReading {
            intent: super::super::approach_model::LineIntent::Examine,
            skill: Some("perception"),
            thing: "bell chamber".to_owned(),
            source: "rules",
            elapsed_ms: 0,
        };
        for context in [
            rpc::DialogueContext::Question,
            rpc::DialogueContext::Social,
            rpc::DialogueContext::PlanOnly,
            rpc::DialogueContext::Meta,
            rpc::DialogueContext::Joke,
        ] {
            let request = rpc::RawDialogueRequest {
                context: context as i32,
                finality: rpc::DialogueFinality::Final as i32,
                ..Default::default()
            };
            assert!(is_observation_submission(&request, &reading).unwrap());
        }
    }
}

impl Service {
    async fn dialogue_call(
        &self,
        credential: [u8; 32],
        request: DialogueRequest,
    ) -> Result<DialogueReply, Status> {
        let (reply, wait) = oneshot::channel();
        self.actor
            .try_submit(actor::Call::Dialogue {
                credential,
                request,
                reply,
            })
            .map_err(|_| Status::resource_exhausted("dialogue actor queue full"))?;
        tokio::time::timeout(super::timing::active().reply, wait)
            .await
            .map_err(|_| Status::deadline_exceeded("retain the confirmed operation for lookup"))?
            .map_err(|_| Status::unavailable("dialogue actor stopped"))?
    }

    pub(super) async fn submit_line(
        &self,
        credential: [u8; 32],
        mut request: rpc::RawDialogueRequest,
    ) -> Result<DialogueReply, Status> {
        let declared = context(request.context)?;
        if request.text.len() > maximum_request_text_bytes(declared) || request.offer_id.len() > 96
        {
            return Err(Status::resource_exhausted(
                "bounded dialogue fields required",
            ));
        }
        let current = self.updates.borrow().clone();
        let scene =
            super::beat_script_runtime::scene(current.state()).map_err(actor::unavailable)?;
        let available_things = scene
            .findings
            .iter()
            .map(|finding| finding.id.clone())
            .chain(scene.cast.iter().map(|person| person.name.clone()))
            .collect::<Vec<_>>();
        let public_scene = format!("{} {}", scene.title, scene.narration);
        let reading = self
            .npc_voice
            .classify_input(&request.text, &public_scene, &available_things)
            .await;
        let input_finality = finality(request.finality)?;
        if input_finality == InputFinality::Final
            && reading.intent == super::approach_model::LineIntent::GameQuestion
        {
            let response = self
                .dialogue_call(
                    credential,
                    DialogueRequest::Raw(encode(&rpc::RawDialogueRequest {
                        context: rpc::DialogueContext::Meta as i32,
                        ..request.clone()
                    })?),
                )
                .await?;
            if let DialogueReply::Raw(mut response, _) = response {
                response.npc_reply =
                    "You can describe what your hero does or ask about something in the scene."
                        .to_owned();
                return Ok(DialogueReply::Raw(response, None));
            }
            return Err(Status::internal("game question response binding invalid"));
        }
        if is_observation_submission(&request, &reading)? {
            return Ok(DialogueReply::Raw(
                self.observation_call(credential, &request, &reading, &scene)
                    .await?,
                None,
            ));
        }
        request.context = match reading.intent {
            super::approach_model::LineIntent::PartyNote => rpc::DialogueContext::Meta as i32,
            super::approach_model::LineIntent::HeroSpeech => rpc::DialogueContext::PlanOnly as i32,
            super::approach_model::LineIntent::GameQuestion => rpc::DialogueContext::Meta as i32,
            super::approach_model::LineIntent::CharacterSpeech => {
                if request.text.trim_end().ends_with('?') {
                    rpc::DialogueContext::Question as i32
                } else {
                    rpc::DialogueContext::Social as i32
                }
            }
            _ => request.context,
        };
        if reading.intent == super::approach_model::LineIntent::PartyNote
            && !request
                .text
                .trim_start()
                .to_lowercase()
                .starts_with("note:")
            && !request
                .text
                .trim_start()
                .to_lowercase()
                .starts_with("write down ")
        {
            request.text = format!("note: {}", request.text);
        }
        self.dialogue_call(credential, DialogueRequest::Raw(encode(&request)?))
            .await
    }

    async fn observation_call(
        &self,
        credential: [u8; 32],
        request: &rpc::RawDialogueRequest,
        reading: &super::approach_model::LineReading,
        scene: &super::pack::Scene,
    ) -> Result<rpc::DialogueResponse, Status> {
        let current = self.updates.borrow().clone();
        let (member_reply, member_wait) = oneshot::channel();
        self.actor
            .try_submit(actor::Call::SpeechMember {
                credential,
                reply: member_reply,
            })
            .map_err(|_| Status::resource_exhausted("dialogue actor queue full"))?;
        let member = tokio::time::timeout(super::timing::active().reply, member_wait)
            .await
            .map_err(|_| Status::deadline_exceeded("dialogue owner did not resolve the player"))?
            .map_err(|_| Status::unavailable("dialogue actor stopped"))??;
        let offers = super::beat_script_runtime::view_offers(&current, member)
            .map_err(actor::unavailable)?;
        let mut observation_offer = None;
        for (kind, _, offer_id) in &offers {
            if *kind == rpc::GameplayActionKind::ExamineHarborSeal
                && super::beat_script_runtime::is_observation_offer(&current, offer_id)
                    .map_err(actor::unavailable)?
            {
                observation_offer = Some(offer_id.clone());
                break;
            }
        }
        let offer_id = observation_offer
            .ok_or_else(|| Status::failed_precondition("this scene has no observation offer"))?;
        let mut action_choices = Vec::new();
        if reading.intent == super::approach_model::LineIntent::WatchPerson {
            let person = scene.cast.iter().find(|person| {
                !reading.thing.is_empty()
                    && reading
                        .thing
                        .to_lowercase()
                        .contains(&person.name.to_lowercase())
            });
            let person = person
                .ok_or_else(|| Status::failed_precondition("name the person you want to watch"))?;
            action_choices.push(rpc::JourneyChoice {
                group_id: "finding".to_owned(),
                option_id: format!("watch-{}", person.id),
            });
            action_choices.push(rpc::JourneyChoice {
                group_id: "attempt-skill".to_owned(),
                option_id: "insight".to_owned(),
            });
        } else if let Some(skill) = reading.skill {
            action_choices.push(rpc::JourneyChoice {
                group_id: "attempt-skill".to_owned(),
                option_id: skill.to_owned(),
            });
        }
        if let Some(thing) = observation_thing_label(&reading.thing) {
            action_choices.push(rpc::JourneyChoice {
                group_id: "attempt-thing".to_owned(),
                option_id: thing,
            });
        }
        let action = rpc::SubmitActionRequest {
            session_id: request.session_id.clone(),
            run_id: request.run_id.clone(),
            observed_revision: request.observed_revision.clone(),
            operation_id: request.input_id.clone(),
            offer_id,
            action_kind: rpc::GameplayActionKind::ExamineHarborSeal as i32,
            action_choices,
            ..Default::default()
        };
        let (reply, wait) = oneshot::channel();
        self.actor
            .try_submit(actor::Call::Submit {
                credential,
                request: action,
                reply,
            })
            .map_err(|_| Status::resource_exhausted("gameplay actor queue full"))?;
        let result = tokio::time::timeout(super::timing::active().reply, wait)
            .await
            .map_err(|_| Status::deadline_exceeded("retain the check operation for lookup"))?
            .map_err(|_| Status::unavailable("gameplay actor stopped"))??;
        let Some(rpc::submit_action_response::Outcome::CommittedDecision(receipt)) = result.outcome
        else {
            return Err(Status::failed_precondition("observation was not committed"));
        };
        let Some(rpc::decision_receipt::Outcome::Accepted(accepted)) = receipt.outcome else {
            return Err(Status::failed_precondition("observation was refused"));
        };
        let check = accepted
            .check
            .as_ref()
            .ok_or_else(|| Status::internal("observation check receipt absent"))?;
        let line = observation_result(&current, scene, check, reading, member, &self.social)?;
        Ok(rpc::DialogueResponse {
            input_id: request.input_id.clone(),
            disposition: rpc::DialogueDisposition::Action as i32,
            confirmation_required: false,
            confirmation_token: Vec::new(),
            revision: receipt.revision,
            npc_reply: line,
        })
    }
}

#[tonic::async_trait]
impl rpc::dialogue_service_server::DialogueService for Service {
    async fn submit(
        &self,
        request: Request<rpc::RawDialogueRequest>,
    ) -> Result<Response<rpc::DialogueResponse>, Status> {
        let credential = credential(&request)?;
        match self.submit_line(credential, request.into_inner()).await? {
            DialogueReply::Raw(mut response, npc) => {
                // The model call runs here, off the actor thread; on any failure the
                // response is exactly the classification the actor produced.
                if let Some(context) = npc {
                    // The table sees the hero's line cleaned up (within its deadline), then
                    // the speaker thinking until the reply (or the lack of one) is in; the
                    // mark clears when this scope ends.
                    let shown = self.npc_voice.display_line(&context).await;
                    context.publish_typed_line(&self.table_log, &shown);
                    let _thinking = self.table_log.reply_pending(&context.speaker);
                    if let Some(reply) = self.npc_voice.converse(context, &self.table_log).await {
                        response.npc_reply = reply;
                    }
                }
                Ok(Response::new(response))
            }
            _ => Err(Status::internal("dialogue response binding invalid")),
        }
    }

    async fn compose(
        &self,
        request: Request<rpc::ComposeRequest>,
    ) -> Result<Response<rpc::ComposeResponse>, Status> {
        let credential = credential(&request)?;
        let kind = rpc::ComposeKind::try_from(request.into_inner().kind)
            .ok()
            .filter(|kind| *kind != rpc::ComposeKind::Unspecified)
            .ok_or_else(|| Status::invalid_argument("known compose kind required"))?;
        let (reply, wait) = oneshot::channel();
        self.actor
            .try_submit(actor::Call::Presence {
                credential,
                kind,
                reply,
            })
            .map_err(|_| Status::resource_exhausted("presence actor queue full"))?;
        tokio::time::timeout(super::timing::active().reply, wait)
            .await
            .map_err(|_| Status::deadline_exceeded("presence not served in time"))?
            .map_err(|_| Status::unavailable("presence actor stopped"))??;
        Ok(Response::new(rpc::ComposeResponse {}))
    }

    async fn confirm(
        &self,
        request: Request<rpc::ConfirmDialogueRequest>,
    ) -> Result<Response<rpc::SubmitActionResponse>, Status> {
        let credential = credential(&request)?;
        let request = request.into_inner();
        if request.confirmation_token.len() != 32 {
            return Err(Status::invalid_argument(
                "exact confirmation token required",
            ));
        }
        match self
            .dialogue_call(credential, DialogueRequest::Confirm(encode(&request)?))
            .await?
        {
            DialogueReply::Confirmed(response) => Ok(Response::new(response)),
            _ => Err(Status::internal("confirmation response binding invalid")),
        }
    }

    async fn cancel(
        &self,
        request: Request<rpc::CancelDialogueRequest>,
    ) -> Result<Response<rpc::DialogueResponse>, Status> {
        let credential = credential(&request)?;
        let request = request.into_inner();
        if request.confirmation_token.len() != 32 {
            return Err(Status::invalid_argument(
                "exact confirmation token required",
            ));
        }
        match self
            .dialogue_call(credential, DialogueRequest::Cancel(encode(&request)?))
            .await?
        {
            DialogueReply::Cancelled(response) => Ok(Response::new(response)),
            _ => Err(Status::internal("cancellation response binding invalid")),
        }
    }
}

fn observation_result(
    current: &df_model::checkpoint::Checkpoint,
    scene: &super::pack::Scene,
    check: &rpc::AbilityCheckResult,
    reading: &super::approach_model::LineReading,
    member: MemberId,
    social: &super::social::SocialLedger,
) -> Result<String, Status> {
    let projection = wire::project_journey(current, LocalDemoRole::Player, member)
        .map_err(actor::unavailable)?;
    let hero = match projection.view.audience {
        Some(rpc::view_message::Audience::Player(player)) => player
            .journey
            .and_then(|journey| journey.own_character)
            .map(|character| character.name),
        _ => None,
    }
    .ok_or_else(|| Status::failed_precondition("hero identity unavailable"))?;
    let pieces = check.source_revision.splitn(4, ':').collect::<Vec<_>>();
    let skill = pieces
        .get(2)
        .copied()
        .ok_or_else(|| Status::internal("observation skill absent"))?;
    let detail = pieces
        .get(3)
        .copied()
        .ok_or_else(|| Status::internal("observation target absent"))?;
    let ability = match skill {
        "perception" | "insight" | "survival" | "medicine" => "Wisdom",
        "investigation" | "arcana" | "history" | "nature" | "religion" => "Intelligence",
        "stealth" => "Dexterity",
        _ => return Err(Status::internal("observation skill invalid")),
    };
    let verb = match skill {
        "stealth" => "scouts",
        "insight" => "watches",
        "survival" => "tracks",
        "arcana" | "history" | "nature" | "religion" | "investigation" => "studies",
        _ => "searches",
    };
    let subject = if reading.thing.is_empty() {
        "the area".to_owned()
    } else {
        reading.thing.clone()
    };
    let modifier = check.ability_modifier
        + i32::try_from(check.proficiency_bonus)
            .map_err(|_| Status::internal("observation proficiency invalid"))?;
    let action_text = match reading.intent {
        super::approach_model::LineIntent::WatchPerson => {
            format!("You watch {subject}.")
        }
        super::approach_model::LineIntent::KeepWatch if skill == "stealth" => {
            format!("You scout {subject}.")
        }
        super::approach_model::LineIntent::KeepWatch => {
            format!("You keep watch {subject}.")
        }
        super::approach_model::LineIntent::Track => format!("You follow {subject}."),
        super::approach_model::LineIntent::StudyRecall => {
            format!("You study {subject}.")
        }
        super::approach_model::LineIntent::Examine => {
            format!("You look around {subject}.")
        }
        _ => "You look around the area.".to_owned(),
    };
    let mut result = format!(
        "{action_text} {hero} {verb} {subject}: {ability} ({}) {} + {modifier} = {}.",
        title_skill(skill),
        check.die,
        check.total,
    );
    let finding = super::beat_script_runtime::observation_finding(scene, check);
    // SRD 5.2.1 p. 187 lists Insight for discerning a creature's state of mind.
    let discovery = if check.succeeded {
        if let Some(finding) = finding {
            finding.sentence.as_str()
        } else if let Some(person_id) = detail.strip_prefix("watch-") {
            let person = scene
                .cast
                .iter()
                .find(|person| person.id == person_id)
                .ok_or_else(|| Status::internal("watched person absent"))?;
            let role = match person.role {
                super::pack::CastRole::Patron => rpc::SceneRole::Patron,
                super::pack::CastRole::Contact => rpc::SceneRole::Contact,
                super::pack::CastRole::Antagonist => rpc::SceneRole::Antagonist,
                super::pack::CastRole::Bystander => rpc::SceneRole::Bystander,
            };
            let Some(dossier) = social.dossier_of(role, &person.id) else {
                return Ok(format!(
                    "{result} You cannot read a change in {}'s manner.",
                    person.name
                ));
            };
            let attitude = social.standing(member, dossier).attitude.label();
            let last_question = social
                .recent(member, &dossier.npc_id)
                .into_iter()
                .rev()
                .find_map(|(is_hero, line)| is_hero.then_some(line));
            let fact = last_question
                .as_deref()
                .map(super::social::subject_keywords)
                .and_then(|subject| dossier.subject_answers.get(&subject))
                .map(|answer| answer.restatement.as_str())
                .unwrap_or("You cannot place their last answer against a known fact.");
            return Ok(format!(
                "{result} {} seems {attitude} toward you. The adventure's facts say: {fact} Their fear is {}.",
                person.name, dossier.fear.text
            ));
        } else {
            return Ok(format!(
                "{result} You see {}: {} Nothing is out of place.",
                scene.title, scene.narration
            ));
        }
    } else {
        "You find nothing unusual."
    };
    result.push(' ');
    result.push_str(discovery);
    Ok(result)
}

fn title_skill(skill: &str) -> &'static str {
    match skill {
        "perception" => "Perception",
        "investigation" => "Investigation",
        "insight" => "Insight",
        "survival" => "Survival",
        "medicine" => "Medicine",
        "arcana" => "Arcana",
        "history" => "History",
        "nature" => "Nature",
        "religion" => "Religion",
        "stealth" => "Stealth",
        _ => "Perception",
    }
}

fn context(value: i32) -> Result<DiscourseContext, Status> {
    Ok(match rpc::DialogueContext::try_from(value) {
        Ok(rpc::DialogueContext::Unspecified) => DiscourseContext::Unspecified,
        Ok(rpc::DialogueContext::Question) => DiscourseContext::Question,
        Ok(rpc::DialogueContext::Social) => DiscourseContext::Social,
        Ok(rpc::DialogueContext::PlanOnly) => DiscourseContext::PlanOnly,
        Ok(rpc::DialogueContext::Meta) => DiscourseContext::Meta,
        Ok(rpc::DialogueContext::Joke) => DiscourseContext::Joke,
        Ok(rpc::DialogueContext::Clarify) => DiscourseContext::Clarify,
        Err(_) => return Err(Status::invalid_argument("unsupported dialogue context")),
    })
}

fn finality(value: i32) -> Result<InputFinality, Status> {
    match rpc::DialogueFinality::try_from(value) {
        Ok(rpc::DialogueFinality::Partial) => Ok(InputFinality::Partial),
        Ok(rpc::DialogueFinality::Final) => Ok(InputFinality::Final),
        _ => Err(Status::invalid_argument("explicit input finality required")),
    }
}

fn disposition(value: IntentDisposition) -> rpc::DialogueDisposition {
    match value {
        IntentDisposition::Action => rpc::DialogueDisposition::Action,
        IntentDisposition::Question => rpc::DialogueDisposition::Question,
        IntentDisposition::Social => rpc::DialogueDisposition::Social,
        IntentDisposition::PlanOnly => rpc::DialogueDisposition::PlanOnly,
        IntentDisposition::Meta => rpc::DialogueDisposition::Meta,
        IntentDisposition::Joke => rpc::DialogueDisposition::Joke,
        IntentDisposition::Clarify => rpc::DialogueDisposition::Clarify,
        IntentDisposition::Rejected => rpc::DialogueDisposition::Rejected,
    }
}

enum NotebookCommand<'a> {
    Note(&'a str),
    Share(u64),
}

fn notebook_command(context: DiscourseContext, text: &str) -> Option<NotebookCommand<'_>> {
    if context != DiscourseContext::Meta {
        return None;
    }
    let text = text.trim();
    let prefix = |value: &str| {
        text.get(..value.len())
            .is_some_and(|found| found.eq_ignore_ascii_case(value))
    };
    if prefix("note:") {
        return Some(NotebookCommand::Note(text.get(5..)?.trim()));
    }
    if prefix("write down") && text.as_bytes().get(10).is_none_or(u8::is_ascii_whitespace) {
        return Some(NotebookCommand::Note(text.get(10..)?.trim()));
    }
    if prefix("share clue ") {
        return text
            .get(11..)?
            .trim()
            .parse()
            .ok()
            .map(NotebookCommand::Share);
    }
    None
}

fn maximum_request_text_bytes(context: DiscourseContext) -> usize {
    if context == DiscourseContext::Meta {
        MAXIMUM_TEXT_BYTES + NOTE_COMMAND_OVERHEAD_BYTES
    } else {
        MAXIMUM_TEXT_BYTES
    }
}

fn dialogue_text_limit(context: DiscourseContext, text: &str) -> usize {
    if text.len() <= maximum_request_text_bytes(context)
        && matches!(notebook_command(context, text), Some(NotebookCommand::Note(note)) if note.len() <= MAXIMUM_TEXT_BYTES)
    {
        text.len()
    } else {
        MAXIMUM_TEXT_BYTES
    }
}

impl actor::Actor {
    pub(super) fn dialogue_member(&mut self, credential: &[u8; 32]) -> Result<MemberId, Status> {
        if self
            .issuer
            .authenticate(credential)
            .map_err(actor::unavailable)?
            != LocalDemoRole::Player
        {
            return Err(Status::permission_denied(
                "player dialogue binding required",
            ));
        }
        let principal = self
            .issuer
            .principal(credential)
            .map_err(actor::unavailable)?;
        if principal == PLAYER {
            return Err(Status::permission_denied(
                "bootstrap is not a dialogue player",
            ));
        }
        let member = MemberId::from_bytes(&principal)
            .map_err(|_| Status::permission_denied("dialogue member binding invalid"))?;
        if !self
            .owner
            .checkpoint()
            .state()
            .members
            .iter()
            .any(|link| link.member == member)
        {
            return Err(Status::permission_denied(
                "current dialogue membership required",
            ));
        }
        Ok(member)
    }

    pub(super) fn handle_dialogue(
        &mut self,
        credential: [u8; 32],
        request: DialogueRequest,
    ) -> Result<DialogueReply, Status> {
        match request {
            DialogueRequest::Raw(bytes) => {
                let request = rpc::RawDialogueRequest::decode(bytes.as_slice())
                    .map_err(|_| Status::invalid_argument("dialogue input invalid"))?;
                self.raw_dialogue(credential, request)
                    .map(|(response, npc)| DialogueReply::Raw(response, npc))
            }
            DialogueRequest::Confirm(bytes) => {
                let request = rpc::ConfirmDialogueRequest::decode(bytes.as_slice())
                    .map_err(|_| Status::invalid_argument("confirmation input invalid"))?;
                self.confirm_dialogue(credential, request)
                    .map(DialogueReply::Confirmed)
            }
            DialogueRequest::Cancel(bytes) => {
                let request = rpc::CancelDialogueRequest::decode(bytes.as_slice())
                    .map_err(|_| Status::invalid_argument("cancellation input invalid"))?;
                self.cancel_dialogue(credential, request)
                    .map(DialogueReply::Cancelled)
            }
        }
    }

    fn raw_dialogue(
        &mut self,
        credential: [u8; 32],
        request: rpc::RawDialogueRequest,
    ) -> Result<(rpc::DialogueResponse, Option<NpcReplyContext>), Status> {
        let member = self.dialogue_member(&credential)?;
        if self.fenced || self.owner.has_uncertain_operation() {
            return Err(Status::unavailable(
                "resolve the retained operation before confirmation",
            ));
        }
        let invalid = || Status::invalid_argument("dialogue scope invalid");
        let input_id = df_api::operation_id(request.input_id.as_ref()).map_err(|_| invalid())?;
        let session = df_api::session_id(request.session_id.as_ref()).map_err(|_| invalid())?;
        let run = df_api::run_id(request.run_id.as_ref()).map_err(|_| invalid())?;
        let observed =
            df_api::session_revision(request.observed_revision.as_ref()).map_err(|_| invalid())?;
        let current = self.owner.checkpoint();
        let basis = current.basis();
        if session != basis.session || run != basis.run || observed != basis.revision {
            return Err(Status::failed_precondition("dialogue basis changed"));
        }
        let finality = finality(request.finality)?;
        let declared = context(request.context)?;
        let reference = RawInputRef {
            input: input_id,
            member,
            basis,
            pins: current.pins().clone(),
            finality,
        };
        let result = classify_dialogue(
            &request.text,
            declared,
            &reference,
            current,
            member,
            DialogueLimits {
                maximum_text_bytes: dialogue_text_limit(declared, &request.text),
            },
        )
        .map_err(|_| Status::invalid_argument("dialogue input rejected"))?;
        let mut token = Vec::new();
        self.dialogue.pending.remove(&member);
        if finality == InputFinality::Final
            && let Some(command) = notebook_command(declared, &request.text)
        {
            match command {
                NotebookCommand::Note(text) => {
                    if text.is_empty() {
                        return Err(Status::invalid_argument("party note text required"));
                    }
                    self.record_notebook_note(member, text)?;
                }
                NotebookCommand::Share(order) => {
                    let shared = self.notebook.share(member, order).map_err(|error| {
                        actor::record_unavailable("notebook-save", &error.to_string());
                        Status::unavailable("notebook state unavailable")
                    })?;
                    let Some(entry) = shared else {
                        return Err(Status::failed_precondition("private clue is not shareable"));
                    };
                    self.table_log.push(
                        "table",
                        rpc::TableRole::Narrator,
                        0,
                        rpc::TableKind::Action,
                        &format!(
                            "NOTE shared kind={} title={:?} scene={}",
                            entry.kind.label(),
                            entry.title,
                            entry.scene
                        ),
                    );
                }
            }
        }
        if result == IntentDisposition::Clarify
            && finality == InputFinality::Final
            && !request.offer_id.is_empty()
        {
            let kind = super::beat_script_runtime::view_offers(current, member)
                .map_err(actor::unavailable)?
                .into_iter()
                .find(|(_, _, id)| *id == request.offer_id)
                .map(|(kind, _, _)| kind)
                .ok_or_else(|| Status::failed_precondition("current offered choice required"))?;
            let charge = current
                .retained_bytes()
                .and_then(|bytes| bytes.checked_add(size_of::<PendingConfirmation>()))
                .and_then(|bytes| bytes.checked_add(request.offer_id.capacity()))
                .ok_or_else(|| Status::resource_exhausted("confirmation capacity overflow"))?;
            let retained = self
                .dialogue
                .pending
                .values()
                .try_fold(charge, |sum, pending| sum.checked_add(pending.charge));
            if self.dialogue.pending.len() >= MAXIMUM_PENDING_MEMBERS
                || retained.is_none_or(|bytes| bytes > MAXIMUM_PENDING_BYTES)
            {
                return Err(Status::resource_exhausted("confirmation capacity exceeded"));
            }
            let proof = model::random::<32>().map_err(actor::unavailable)?;
            token = proof.to_vec();
            self.dialogue.pending.insert(
                member,
                PendingConfirmation {
                    input: reference,
                    token: proof,
                    offer: request.offer_id,
                    kind,
                    selection: None,
                    charge,
                },
            );
        }
        // Conversation addressed to the scene's speaker is answered in character; a pending
        // confirmation ticket means the player is clarifying an action, which is not.
        let npc = (token.is_empty()
            && finality == InputFinality::Final
            && matches!(
                result,
                IntentDisposition::Question
                    | IntentDisposition::Social
                    | IntentDisposition::Joke
                    | IntentDisposition::Clarify
            ))
        .then(|| self.npc_reply_context(member, &request.text))
        .flatten();
        if finality == InputFinality::Final
            && declared != DiscourseContext::Meta
            && result != IntentDisposition::Meta
            && npc.is_none()
            && let Ok(projection) = wire::project_journey(current, LocalDemoRole::Player, member)
            && let Some(rpc::view_message::Audience::Player(player)) = projection.view.audience
            && let Some(journey) = player.journey
            && let Some(hero) = journey.own_character.as_ref()
        {
            let hero_index = journey
                .party
                .iter()
                .position(|party_member| party_member.name == hero.name)
                .and_then(|position| u32::try_from(position).ok())
                .unwrap_or(0);
            self.table_log
                .push_speech(&hero.name, hero_index, &request.text);
        }
        // Typed words to the scene's speaker are table talk, answered or not: the service
        // publishes their display version, cleaned off the actor thread.
        Ok((
            rpc::DialogueResponse {
                input_id: request.input_id,
                disposition: disposition(result) as i32,
                confirmation_required: !token.is_empty(),
                confirmation_token: token,
                revision: Some(wire::revision(basis.revision)),
                npc_reply: String::new(),
            },
            npc,
        ))
    }

    /// The facts for the speaker's reply, taken from this member's own view. Failing to build
    /// them only means this line gets today's response without a spoken reply.
    fn npc_reply_context(&self, member: MemberId, typed: &str) -> Option<NpcReplyContext> {
        let projection =
            wire::project_journey(self.owner.checkpoint(), LocalDemoRole::Player, member).ok()?;
        let story = super::pack::with_content(StoryFacts::from_pack);
        NpcReplyContext::for_cast(
            &projection.view,
            member,
            typed,
            &story,
            &self.social,
            &format!("{:?}", projection.scene),
        )
    }

    fn record_notebook_note(&self, member: MemberId, text: &str) -> Result<(), Status> {
        let current = self.owner.checkpoint();
        let projection = wire::project_journey(current, LocalDemoRole::Player, member)
            .map_err(actor::unavailable)?;
        let Some(rpc::view_message::Audience::Player(player)) = projection.view.audience else {
            return Err(Status::failed_precondition("player view required"));
        };
        let hero = player
            .journey
            .and_then(|journey| journey.own_character)
            .map(|character| character.name)
            .ok_or_else(|| Status::failed_precondition("hero required for party note"))?;
        let scene =
            super::beat_script_runtime::scene(current.state()).map_err(actor::unavailable)?;
        let scene = super::notebook::scene_name(scene.key)
            .map_err(|_| Status::internal("notebook scene unavailable"))?;
        self.notebook
            .add_note(member, &hero, text, &scene)
            .map_err(|error| match error {
                super::notebook::NotebookError::Bound("text") => {
                    Status::resource_exhausted("party note text exceeds its bound")
                }
                super::notebook::NotebookError::Io(_) => {
                    actor::record_unavailable("notebook-save", &error.to_string());
                    Status::unavailable("notebook state unavailable")
                }
                _ => Status::invalid_argument("party note rejected"),
            })?;
        self.table_log.push(
            "table",
            rpc::TableRole::Narrator,
            0,
            rpc::TableKind::Action,
            &format!("NOTE added kind=note title=\"Party note\" scene={scene}"),
        );
        Ok(())
    }

    fn confirm_dialogue(
        &mut self,
        credential: [u8; 32],
        request: rpc::ConfirmDialogueRequest,
    ) -> Result<rpc::SubmitActionResponse, Status> {
        let member = self.dialogue_member(&credential)?;
        let input_id = df_api::operation_id(request.input_id.as_ref())
            .map_err(|_| Status::invalid_argument("confirmation input identity required"))?;
        let action = request
            .action
            .ok_or_else(|| Status::invalid_argument("separate typed choice required"))?;
        if action.offer_id.len() > 96
            || action.character.as_ref().is_some_and(|character| {
                character.name.len() > 48
                    || character.choices.len() > 16
                    || character
                        .choices
                        .iter()
                        .any(|choice| choice.group_id.len() > 64 || choice.option_id.len() > 64)
            })
        {
            return Err(Status::invalid_argument("bounded typed choice required"));
        }
        let operation = df_api::operation_id(action.operation_id.as_ref())
            .map_err(|_| Status::invalid_argument("confirmed operation identity required"))?;
        if operation == input_id {
            return Err(Status::invalid_argument(
                "confirmation is a separate user operation",
            ));
        }
        let fingerprint: [u8; 32] = Sha256::digest(action.encode_to_vec()).into();
        let pending = self
            .dialogue
            .pending
            .get_mut(&member)
            .ok_or_else(|| Status::failed_precondition("confirmation expired or cancelled"))?;
        if pending.input.input != input_id
            || request.confirmation_token.as_slice() != pending.token.as_slice()
            || action.offer_id != pending.offer
            || action.action_kind != pending.kind as i32
        {
            return Err(Status::permission_denied("confirmation binding mismatch"));
        }
        if let Some(selection) = pending.selection {
            if selection != (fingerprint, operation) {
                return Err(Status::failed_precondition(
                    "exact confirmed operation retry required",
                ));
            }
            // A previously admitted exact retry reaches native lookup before
            // current offer/basis checks. Session owns uncertainty and receipts.
        } else {
            confirm_final_input(&pending.input, self.owner.checkpoint(), member)
                .map_err(|_| Status::failed_precondition("final input or source basis changed"))?;
            if self.fenced || self.owner.has_uncertain_operation() {
                return Err(Status::unavailable("resolve the retained operation first"));
            }
            let current = self.owner.checkpoint();
            if !journey::offered(current, member)
                .map_err(actor::unavailable)?
                .iter()
                .any(|(kind, _)| *kind == pending.kind)
                || !super::beat_script_runtime::matches_id(current, pending.kind, &pending.offer)
                    .map_err(actor::unavailable)?
            {
                return Err(Status::failed_precondition("confirmed offer changed"));
            }
            let command = wire::journey_input(&action, member, current)?;
            let df_model::checkpoint::GameInput::Game(command) = command else {
                return Err(Status::invalid_argument("explicit typed choice required"));
            };
            if command.basis != pending.input.basis
                || command.observed_revision != pending.input.basis.revision
            {
                return Err(Status::failed_precondition(
                    "confirmed selection basis changed",
                ));
            }
            pending.selection = Some((fingerprint, operation));
        }
        self.submit(credential, action)
    }

    fn cancel_dialogue(
        &mut self,
        credential: [u8; 32],
        request: rpc::CancelDialogueRequest,
    ) -> Result<rpc::DialogueResponse, Status> {
        let member = self.dialogue_member(&credential)?;
        let input_id = df_api::operation_id(request.input_id.as_ref())
            .map_err(|_| Status::invalid_argument("cancellation input identity required"))?;
        let pending = self
            .dialogue
            .pending
            .get(&member)
            .ok_or_else(|| Status::failed_precondition("confirmation expired or cancelled"))?;
        if pending.input.input != input_id
            || request.confirmation_token.as_slice() != pending.token.as_slice()
        {
            return Err(Status::permission_denied("cancellation binding mismatch"));
        }
        if pending.selection.is_some() {
            return Err(Status::failed_precondition(
                "confirmed operation cannot be cancelled; retry exact confirmation",
            ));
        }
        self.dialogue.pending.remove(&member);
        Ok(rpc::DialogueResponse {
            input_id: request.input_id,
            disposition: rpc::DialogueDisposition::Rejected as i32,
            confirmation_token: Vec::new(),
            confirmation_required: false,
            revision: Some(wire::revision(self.owner.checkpoint().basis().revision)),
            npc_reply: String::new(),
        })
    }
}
