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
    pub(super) source_line_id: String,
    pub(super) revision: String,
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
}

impl fmt::Display for ClassificationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnknownLabel => "jev returned an unknown NPC event label",
            Self::UnknownItem => "jev selected an item that is not in the current scene",
            Self::MissingItem => "jev selected an item event without an item",
        })
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
            .map(|offer| (offer.id.clone(), offer.name.clone())),
    );
    choices.sort_by(|left, right| left.0.cmp(&right.0));
    choices.dedup_by(|left, right| left.0 == right.0);
    choices
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
    source_line_id: String,
    revision: String,
    items: &[(String, String)],
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
    Ok(Proposal {
        kind,
        actor_id: actor_id.to_owned(),
        npc_id: npc_id.to_owned(),
        item_id,
        source_line_id,
        revision,
    })
}

pub(super) fn provider_state(
    actor_id: &str,
    npc_id: &str,
    revision: &str,
    reply: &str,
    items: &[(String, String)],
) -> serde_json::Value {
    json!({
        "actor_id": actor_id,
        "npc_id": npc_id,
        "current_revision": revision,
        "npc_reply": reply,
        "canonical_items": items.iter().map(|(id, name)| json!({"id": id, "name": name})).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<(String, String)> {
        vec![("cloth-1".to_owned(), "fine cloak".to_owned())]
    }

    #[test]
    fn accepts_only_canonical_item_ids_and_preserves_revision() {
        let result = proposal(
            "offer:cloth-1",
            "actor-marek",
            "marek",
            "npc-line:test".to_owned(),
            "3:8".to_owned(),
            &items(),
        )
        .unwrap();
        assert_eq!(result.kind, EventKind::Offer);
        assert_eq!(result.actor_id, "actor-marek");
        assert_eq!(result.item_id.as_deref(), Some("cloth-1"));
        assert_eq!(result.revision, "3:8");
    }

    #[test]
    fn refuses_unknown_ids_and_unqualified_item_events() {
        assert_eq!(
            proposal(
                "intent_to_take:missing",
                "actor-marek",
                "marek",
                "line".to_owned(),
                "1:1".to_owned(),
                &items(),
            ),
            Err(ClassificationError::UnknownItem)
        );
        assert_eq!(
            proposal(
                "offer",
                "actor-marek",
                "marek",
                "line".to_owned(),
                "1:1".to_owned(),
                &items(),
            ),
            Err(ClassificationError::MissingItem)
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
