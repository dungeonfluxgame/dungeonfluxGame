//! Jev classification of an NPC's already-produced reply.
//!
//! This module only emits a typed proposal.  It cannot mutate a scene, grant an item,
//! authorize a payment, or create a fact.  The session/location owner must validate the
//! canonical IDs and commit the resulting event against the current revision.

use std::fmt;

use serde_json::json;
use sha2::{Digest, Sha256};

use super::adventure::scene_model::SceneNow;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum EventKind {
    None,
    Reveal,
    Offer,
    IntentToBuy,
    IntentToTake,
    IntentToGive,
    Refusal,
    Conditional,
}

impl EventKind {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Reveal => "reveal",
            Self::Offer => "offer",
            Self::IntentToBuy => "intent_to_buy",
            Self::IntentToTake => "intent_to_take",
            Self::IntentToGive => "intent_to_give",
            Self::Refusal => "refusal",
            Self::Conditional => "conditional",
        }
    }

    fn from_label(label: &str) -> Option<Self> {
        Some(match label {
            "none" => Self::None,
            "reveal" => Self::Reveal,
            "offer" => Self::Offer,
            "intent_to_buy" => Self::IntentToBuy,
            "intent_to_take" => Self::IntentToTake,
            "intent_to_give" => Self::IntentToGive,
            "refusal" => Self::Refusal,
            "conditional" => Self::Conditional,
            _ => return None,
        })
    }

    fn requires_item(self) -> bool {
        matches!(
            self,
            Self::Reveal
                | Self::Offer
                | Self::IntentToBuy
                | Self::IntentToTake
                | Self::IntentToGive
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Proposal {
    pub(super) kind: EventKind,
    pub(super) actor_id: String,
    pub(super) npc_id: String,
    pub(super) item_id: Option<String>,
    /// Canonical recipient for an explicit, present handoff. This is filled only when the
    /// current scene contains the addressed hero; a future or conditional offer leaves it empty.
    pub(super) recipient_id: Option<String>,
    /// The bounded, already-spoken line used to bind the proposal to the current table log.
    pub(super) reply_text: String,
    pub(super) typed_words: String,
    pub(super) scene_identity: String,
    pub(super) source_line_id: String,
    /// Assigned when the reply is appended to the public table log.
    pub(super) source_line_sequence: Option<u64>,
    pub(super) revision: String,
    /// A deterministic, source-bound merchant price proposal. The same NPC event route validates
    /// its public source and persists it through the inventory owner's quote journal.
    pub(super) trade_quote: Option<super::journey::merchant_bargaining::QuoteCandidate>,
}

impl Proposal {
    pub(super) fn source_line_id(
        actor_id: &str,
        npc_id: &str,
        scene_identity: &str,
        revision: &str,
        typed_words: &str,
        reply: &str,
    ) -> String {
        let mut digest = Sha256::new();
        digest.update(b"dungeonflux:npc-reply-event:v1\0");
        for value in [
            actor_id,
            npc_id,
            scene_identity,
            revision,
            typed_words,
            reply,
        ] {
            digest.update(value.as_bytes());
            digest.update([0]);
        }
        format!("npc-line:{:x}", digest.finalize())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ClassificationError {
    UnknownLabel,
    UnknownItem,
    MissingItem,
    ItemNotInRequest,
    ItemNotInReply,
    WrongItemInReply,
    // Never constructed now that Jev's intent_to_give decides handoff presence; approach_model.rs
    // still matches this label exhaustively, so the variant stays.
    #[allow(dead_code)]
    HandoffNotPresent,
}

impl fmt::Display for ClassificationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnknownLabel => "jev returned an unknown NPC event label",
            Self::UnknownItem => "jev selected an item that is not in the current scene",
            Self::MissingItem => "jev selected an item event without an item",
            Self::ItemNotInRequest => "jev selected an item the player did not name",
            Self::ItemNotInReply => "the NPC reply does not identify the selected item",
            Self::WrongItemInReply => "the NPC reply identifies a different canonical item",
            Self::HandoffNotPresent => "the NPC reply does not describe a present physical handoff",
        })
    }
}

impl ClassificationError {
    pub(super) fn label(&self) -> &'static str {
        match self {
            Self::UnknownLabel => "unknown_label",
            Self::UnknownItem => "unknown_item",
            Self::MissingItem => "missing_item",
            Self::ItemNotInRequest => "item_not_in_request",
            Self::ItemNotInReply => "item_not_in_reply",
            Self::WrongItemInReply => "wrong_item_in_reply",
            Self::HandoffNotPresent => "handoff_not_present",
        }
    }
}

/// Canonical item choices supplied to Jev.  Names are context only; IDs are the only values
/// that may cross into the owner validation path.
pub(super) fn item_choices(room: &SceneNow) -> Vec<(String, String)> {
    let mut choices = room
        .model
        .things
        .iter()
        .filter(|thing| thing.in_scene)
        .map(|thing| (thing.id.clone(), thing.name.clone()))
        .collect::<Vec<_>>();
    choices.extend(
        room.model
            .offers
            .iter()
            .map(|offer| (offer.item_id.clone(), offer.name.clone())),
    );
    choices.sort_by(|left, right| left.0.cmp(&right.0));
    choices.dedup_by(|left, right| left.0 == right.0);
    choices
}

fn item_anchors(room: &SceneNow, item_id: &str) -> Vec<String> {
    if let Some(thing) = room
        .model
        .things
        .iter()
        .find(|thing| thing.id == item_id && thing.in_scene)
    {
        return std::iter::once(thing.name.clone())
            .chain(std::iter::once(thing.short.clone()))
            .chain(thing.aliases.iter().cloned())
            .filter(|value| !value.trim().is_empty())
            .collect();
    }
    room.model
        .offers
        .iter()
        .find(|offer| offer.item_id == item_id)
        .map(|offer| {
            std::iter::once(offer.name.clone())
                .chain(offer.aliases.iter().cloned())
                .filter(|value| !value.trim().is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn normalized_words(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn contains_anchor(text: &str, anchor: &str) -> bool {
    let text = normalized_words(text);
    let anchor = normalized_words(anchor);
    !anchor.is_empty() && format!(" {text} ").contains(&format!(" {anchor} "))
}

fn mentions_item(text: &str, room: &SceneNow, item_id: &str) -> bool {
    item_anchors(room, item_id)
        .iter()
        .any(|anchor| contains_anchor(text, anchor))
}

/// Revalidates the exact grounding before an item proposal can cross the actor boundary: a known
/// canonical ID, the item named in the player's request, and no other item named in the reply.
/// Whether the reply is a present handoff is Jev's judgement in `intent_to_give`, not a phrase list.
pub(super) fn validate_item_event(
    kind: EventKind,
    item_id: &str,
    typed_words: &str,
    reply: &str,
    room: &SceneNow,
) -> Result<(), ClassificationError> {
    let known = item_choices(room);
    if !known.iter().any(|(known_id, _)| known_id == item_id) {
        return Err(ClassificationError::UnknownItem);
    }
    if matches!(
        kind,
        EventKind::IntentToBuy | EventKind::IntentToTake | EventKind::IntentToGive
    ) && !mentions_item(typed_words, room, item_id)
    {
        return Err(ClassificationError::ItemNotInRequest);
    }

    let reply_items = known
        .iter()
        .filter(|(id, _)| mentions_item(reply, room, id))
        .map(|(id, _)| id.as_str())
        .collect::<Vec<_>>();
    if reply_items.iter().any(|mentioned| *mentioned != item_id) {
        return Err(ClassificationError::WrongItemInReply);
    }
    // A deictic handoff ("here, have a look") names no item, so an intent_to_give reply needs no
    // anchor here: Jev chose this item from the request, and the wrong-item check above applies.
    if kind != EventKind::IntentToGive && !mentions_item(reply, room, item_id) {
        return Err(ClassificationError::ItemNotInReply);
    }
    Ok(())
}

pub(super) fn criteria(items: &[(String, String)]) -> Vec<String> {
    choice_labels(items)
        .into_iter()
        .map(|label| match label.split_once(':') {
            Some((kind, _id)) => match kind {
                "reveal" => format!("{label}: the NPC newly points out or makes this present item salient"),
                "offer" => format!("{label}: the NPC explicitly offers this present item to someone"),
                "intent_to_buy" => format!("{label}: the NPC explicitly says they intend to buy this item"),
                "intent_to_take" => format!("{label}: the NPC explicitly says they intend to take this item"),
                "intent_to_give" => format!("{label}: the NPC explicitly says they intend to give this item"),
                _ => format!("{kind}: the NPC's reply has this disposition"),
            },
            None => match label.as_str() {
                "none" => format!("{label}: the reply is ordinary conversation and proposes no world event"),
                "refusal" => format!("{label}: the NPC declines, withholds, or says they will not do or reveal something"),
                "conditional" => format!("{label}: the NPC describes a possible action only if a condition is met; do not treat it as an offer or commitment"),
                _ => format!("{label}: the NPC's reply has this disposition"),
            },
        })
        .collect()
}

pub(super) fn choice_labels(items: &[(String, String)]) -> Vec<String> {
    let mut result = vec![
        "none".to_owned(),
        "refusal".to_owned(),
        "conditional".to_owned(),
    ];
    for (id, _) in items {
        result.extend([
            format!("reveal:{id}"),
            format!("offer:{id}"),
            format!("intent_to_buy:{id}"),
            format!("intent_to_take:{id}"),
            format!("intent_to_give:{id}"),
        ]);
    }
    result
}

/// Parses and validates the closed Jev choice against the scene's current canonical IDs.
pub(super) fn proposal(
    choice: &str,
    actor_id: &str,
    npc_id: &str,
    scene_identity: String,
    source_line_id: String,
    revision: String,
    typed_words: String,
    reply_text: String,
    items: &[(String, String)],
    room: &SceneNow,
) -> Result<Proposal, ClassificationError> {
    let (label, item_id) = choice
        .split_once(':')
        .map_or((choice, None), |(kind, id)| (kind, Some(id)));
    let kind = EventKind::from_label(label).ok_or(ClassificationError::UnknownLabel)?;
    if kind.requires_item() && item_id.is_none() {
        return Err(ClassificationError::MissingItem);
    }
    let item_id = item_id.map(str::to_owned);
    if let Some(id) = &item_id
        && !items.iter().any(|(known, _)| known == id)
    {
        return Err(ClassificationError::UnknownItem);
    }
    if let Some(item_id) = &item_id {
        validate_item_event(kind, item_id, &typed_words, &reply_text, room)?;
    }
    Ok(Proposal {
        kind,
        actor_id: actor_id.to_owned(),
        npc_id: npc_id.to_owned(),
        item_id,
        recipient_id: None,
        reply_text,
        typed_words,
        scene_identity,
        source_line_id,
        source_line_sequence: None,
        revision,
        trade_quote: None,
    })
}

pub(super) fn provider_state(
    actor_id: &str,
    npc_id: &str,
    scene_identity: &str,
    revision: &str,
    typed_words: &str,
    reply: &str,
    items: &[(String, String)],
) -> serde_json::Value {
    json!({
        "actor_id": actor_id,
        "npc_id": npc_id,
        "scene_identity": scene_identity,
        "current_revision": revision,
        "player_words": typed_words,
        "npc_reply": reply,
        "canonical_items": items.iter().map(|(id, name)| json!({"id": id, "name": name})).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gameplay::{adventure::scene_model::*, pack::FindingSkill};

    fn thing(id: &str, name: &str, aliases: &[&str]) -> SceneThing {
        SceneThing {
            id: id.to_owned(),
            name: name.to_owned(),
            short: name.to_owned(),
            detail: format!("A {name} for inspection."),
            picture_description: name.to_owned(),
            closer_look: format!("The {name} has a careful weave."),
            skill: FindingSkill::Perception,
            dc: 10,
            finding_id: None,
            clue_fact_id: None,
            portable: true,
            value_cp: None,
            use_effect: String::new(),
            position: "counter".to_owned(),
            owner_id: Some("merchant".to_owned()),
            in_use: false,
            in_scene: true,
            publicly_visible: false,
            holder_id: None,
            open: false,
            lit: false,
            broken: false,
            searched_by: Vec::new(),
            aliases: aliases.iter().map(|alias| (*alias).to_owned()).collect(),
            provenance: SceneItemProvenance {
                kind: SceneItemProvenanceKind::AuthoredLocation,
                source_id: id.to_owned(),
                seller_id: None,
            },
        }
    }

    fn room() -> SceneNow {
        SceneNow {
            model: SceneModel {
                witnessed_thefts: Vec::new(),
                inventory_version: 1,
                time_of_day: "day".to_owned(),
                weather: "clear".to_owned(),
                people: Vec::new(),
                things: vec![
                    thing(
                        "item-clothes-travelers",
                        "traveler's clothes",
                        &["fine clothes"],
                    ),
                    thing("item-brass-hand-bell", "brass hand bell", &["hand bell"]),
                    thing(
                        "item-brass-key",
                        "brass key",
                        &["key wrapped in blue cloth"],
                    ),
                ],
                ways_out: Vec::new(),
                offers: Vec::new(),
            },
            changes: Vec::new(),
        }
    }

    fn items(room: &SceneNow) -> Vec<(String, String)> {
        item_choices(room)
    }

    #[test]
    fn accepts_only_canonical_item_ids_and_preserves_revision() {
        let room = room();
        let result = proposal(
            "offer:item-clothes-travelers",
            "actor-marek",
            "marek",
            "shop|plate".to_owned(),
            "npc-line:test".to_owned(),
            "3:8".to_owned(),
            "show me the fine clothes".to_owned(),
            "The traveler's clothes have a careful weave.".to_owned(),
            &items(&room),
            &room,
        )
        .unwrap();
        assert_eq!(result.kind, EventKind::Offer);
        assert_eq!(result.actor_id, "actor-marek");
        assert_eq!(result.item_id.as_deref(), Some("item-clothes-travelers"));
        assert_eq!(result.revision, "3:8");
        assert_eq!(result.typed_words, "show me the fine clothes");
        assert_eq!(result.scene_identity, "shop|plate");
    }

    #[test]
    fn refuses_unknown_ids_and_unqualified_item_events() {
        let room = room();
        assert_eq!(
            proposal(
                "intent_to_take:missing",
                "actor-marek",
                "marek",
                "shop|plate".to_owned(),
                "line".to_owned(),
                "1:1".to_owned(),
                "take the fine clothes".to_owned(),
                "Here, take it.".to_owned(),
                &items(&room),
                &room,
            ),
            Err(ClassificationError::UnknownItem)
        );
        assert_eq!(
            proposal(
                "offer",
                "actor-marek",
                "marek",
                "shop|plate".to_owned(),
                "line".to_owned(),
                "1:1".to_owned(),
                "take the fine clothes".to_owned(),
                "The traveler's clothes look sturdy.".to_owned(),
                &items(&room),
                &room,
            ),
            Err(ClassificationError::MissingItem)
        );
    }

    #[test]
    fn accepts_a_source_grounded_deictic_inspection_handoff() {
        let room = room();
        assert_eq!(
            validate_item_event(
                EventKind::IntentToGive,
                "item-clothes-travelers",
                "Marek, may I hold the fine clothes for a moment?",
                "Here, have a look.",
                &room,
            ),
            Ok(())
        );
    }

    #[test]
    fn rejects_the_brass_key_reply_for_a_brass_hand_bell_request() {
        let room = room();
        assert_eq!(
            validate_item_event(
                EventKind::IntentToGive,
                "item-brass-hand-bell",
                "Marek, may I hold the brass hand bell for a moment?",
                "The brass key is a brass key wrapped in blue cloth.",
                &room,
            ),
            Err(ClassificationError::WrongItemInReply)
        );
    }

    #[test]
    fn requires_the_item_to_be_anchored_in_the_player_request() {
        let room = room();
        assert_eq!(
            validate_item_event(
                EventKind::IntentToGive,
                "item-brass-hand-bell",
                "Here, may I hold that for a moment?",
                "Here, take it.",
                &room,
            ),
            Err(ClassificationError::ItemNotInRequest)
        );
    }

    #[test]
    fn line_identity_is_stable_and_changes_when_reply_changes() {
        let first = Proposal::source_line_id(
            "actor-marek",
            "marek",
            "shop|plate",
            "1:2",
            "hello",
            "welcome",
        );
        assert_eq!(
            first,
            Proposal::source_line_id(
                "actor-marek",
                "marek",
                "shop|plate",
                "1:2",
                "hello",
                "welcome",
            )
        );
        assert_ne!(
            first,
            Proposal::source_line_id(
                "actor-marek",
                "marek",
                "shop|plate",
                "1:2",
                "hello",
                "different",
            )
        );
    }
}
