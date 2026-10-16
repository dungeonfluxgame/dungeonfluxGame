//! Public narrative focus selects existing subjects; it never proposes a world-state change.

use super::adventure::scene_model::SceneModel;
use serde::Serialize;
use std::time::Duration;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum Subject {
    Offer {
        id: String,
        name: String,
        description: String,
        seller_id: String,
        cost_cp: u32,
    },
    Thing {
        id: String,
        name: String,
        description: String,
        position: String,
        owner_id: Option<String>,
        holder_id: Option<String>,
    },
}

impl Subject {
    pub(super) fn id(&self) -> &str {
        match self {
            Self::Offer { id, .. } | Self::Thing { id, .. } => id,
        }
    }

    pub(super) fn kind(&self) -> &'static str {
        match self {
            Self::Offer { .. } => "offer",
            Self::Thing { .. } => "thing",
        }
    }

    pub(super) fn remains_current(&self, model: &SceneModel) -> bool {
        match self {
            Self::Offer {
                id,
                name,
                description,
                seller_id,
                cost_cp,
            } => model.offers.iter().any(|offer| {
                offer.id == *id
                    && model.offer_available(id)
                    && offer.name == *name
                    && offer.description == *description
                    && offer.seller_id == *seller_id
                    && offer.cost_cp == *cost_cp
            }),
            Self::Thing {
                id,
                name,
                description,
                holder_id,
                owner_id,
                position,
            } => model.things.iter().any(|thing| {
                thing.id == *id
                    && thing.in_scene
                    && thing.publicly_visible
                    && thing.name == *name
                    && thing.picture_description == *description
                    && thing.holder_id == *holder_id
                    && thing.owner_id == *owner_id
                    && thing.position == *position
            }),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct NarrativeFocus {
    pub(super) dialogue_sequence: u64,
    pub(super) speaker_actor_id: String,
    pub(super) subject: Subject,
}

/// A complete, public, committed NPC line. The caller verifies the speaker's current cast ID.
pub(super) struct Dialogue<'a> {
    pub(super) sequence: u64,
    pub(super) actor_id: &'a str,
    pub(super) text: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Stage {
    Selected,
    Queued,
    Requested,
    Published,
    Failed,
}

/// Owned by the plate director. This is presentation progress, never a stock or inventory row.
pub(super) struct Presentation {
    pub(super) focus: NarrativeFocus,
    pub(super) request_revision: u64,
    pub(super) published_revision: Option<u64>,
    pub(super) image_key: Option<String>,
    pub(super) stage: Stage,
    pub(super) selected_at: Duration,
    pub(super) elapsed_ms: u128,
}

impl Presentation {
    pub(super) fn public_view(&self) -> serde_json::Value {
        serde_json::json!({
            "focus": self.focus,
            "revision_kind": "scene_graph",
            "request_revision": self.request_revision,
            "published_revision": self.published_revision,
            "image_key": self.image_key,
            "stage": self.stage,
            "elapsed_ms": self.elapsed_ms,
            "actions": match &self.focus.subject {
                Subject::Offer { .. } => vec!["look", "buy"],
                Subject::Thing { .. } => vec!["look"],
            },
        })
    }
}

/// Match canonical names and aliases as whole word spans, ignoring case. Prefer the most
/// specific alias; equal matches for different subjects leave the camera unchanged.
pub(super) fn select(model: &SceneModel, dialogue: Dialogue<'_>) -> Option<NarrativeFocus> {
    let words = words(dialogue.text);
    let mut candidates = Vec::new();
    for offer in &model.offers {
        if !model.offer_available(&offer.id)
            || !model
                .people
                .iter()
                .any(|person| person.id == offer.seller_id)
        {
            continue;
        }
        if let Some(specificity) = mentioned(&words, &offer.name, &offer.aliases) {
            candidates.push((
                specificity,
                Subject::Offer {
                    id: offer.id.clone(),
                    name: offer.name.clone(),
                    description: offer.description.clone(),
                    seller_id: offer.seller_id.clone(),
                    cost_cp: offer.cost_cp,
                },
            ));
        }
    }
    for thing in model
        .things
        .iter()
        .filter(|thing| thing.in_scene && thing.publicly_visible)
    {
        if let Some(specificity) = mentioned(&words, &thing.name, &thing.aliases) {
            candidates.push((
                specificity,
                Subject::Thing {
                    id: thing.id.clone(),
                    name: thing.name.clone(),
                    description: thing.picture_description.clone(),
                    position: thing.position.clone(),
                    owner_id: thing.owner_id.clone(),
                    holder_id: thing.holder_id.clone(),
                },
            ));
        }
    }
    let most_specific = candidates
        .iter()
        .map(|(specificity, _)| *specificity)
        .max()?;
    let matches = candidates
        .into_iter()
        .filter(|(specificity, _)| *specificity == most_specific);
    let mut matches = matches.collect::<Vec<_>>();
    let held_thing = matches.iter().position(|(_, subject)| {
        matches!(
            subject,
            Subject::Thing {
                holder_id: Some(_),
                ..
            }
        )
    });
    if let Some(index) = held_thing
        && matches
            .iter()
            .enumerate()
            .all(|(other, (_, subject))| other == index || matches!(subject, Subject::Offer { .. }))
    {
        let (_, subject) = matches.swap_remove(index);
        return Some(NarrativeFocus {
            dialogue_sequence: dialogue.sequence,
            speaker_actor_id: dialogue.actor_id.to_owned(),
            subject,
        });
    }
    if matches.len() != 1 {
        return None;
    }
    let (_, subject) = matches.pop()?;
    Some(NarrativeFocus {
        dialogue_sequence: dialogue.sequence,
        speaker_actor_id: dialogue.actor_id.to_owned(),
        subject,
    })
}

fn words(text: &str) -> Vec<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn mentioned(text: &[String], name: &str, aliases: &[String]) -> Option<usize> {
    std::iter::once(name)
        .chain(aliases.iter().map(String::as_str))
        .filter_map(|alias| {
            let alias = words(alias);
            (!alias.is_empty() && text.windows(alias.len()).any(|span| span == alias))
                .then(|| alias.iter().map(String::len).sum())
        })
        .max()
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::gameplay::adventure::{
        scene_model::ScenePlace,
        setting::{SceneSetting, SettingKind},
    };
    use std::collections::BTreeMap;

    pub(in crate::gameplay) fn shop() -> SceneModel {
        SceneModel::fallback(
            &SceneSetting {
                place_id: "cloth-shop".into(),
                name: "the cloth shop".into(),
                short: "the cloth shop".into(),
                kind: SettingKind::Shop,
                exterior: false,
                art_key: "place-cloth-shop".into(),
                model: None,
            },
            ScenePlace::Meeting,
            671003,
            &BTreeMap::from([
                ("stake.short".into(), "the afternoon's trade".into()),
                ("patron.full".into(), "Marek".into()),
                ("patron.role".into(), "cloth merchant".into()),
                (
                    "patron.desc".into(),
                    "a cloth merchant behind the counter".into(),
                ),
            ]),
        )
        .unwrap()
    }

    #[test]
    fn committed_merchant_price_line_focuses_actual_fine_clothes_without_a_sale() {
        let model = shop();
        let before = serde_json::to_value(&model).unwrap();
        let focus = select(
            &model,
            Dialogue {
                sequence: 42,
                actor_id: "merchant",
                text: "The fine clothes are fifteen hundred copper.",
            },
        )
        .unwrap();
        assert_eq!(focus.dialogue_sequence, 42);
        assert_eq!(focus.subject.id(), "clothes-fine");
        assert!(
            matches!(&focus.subject, Subject::Offer { cost_cp: 1500, seller_id, .. } if seller_id == "patron")
        );
        assert_eq!(serde_json::to_value(&model).unwrap(), before);
        assert!(
            !model
                .things
                .iter()
                .any(|thing| thing.id == "purchased-clothes-fine")
        );
    }

    #[test]
    fn exact_canonical_aliases_ignore_case_and_never_create_unknown_subjects() {
        let model = shop();
        for line in [
            "Fine CLOTHES?",
            "What if the FINE CLOTHES burned tomorrow?",
            "The formal clothes are here.",
        ] {
            assert_eq!(
                select(
                    &model,
                    Dialogue {
                        sequence: 4,
                        actor_id: "merchant",
                        text: line
                    }
                )
                .unwrap()
                .subject
                .id(),
                "clothes-fine"
            );
        }
        for line in [
            "We could buy a dragon tomorrow.",
            "Fine clothesline rope.",
            "What if someone attacked the shop?",
        ] {
            assert!(
                select(
                    &model,
                    Dialogue {
                        sequence: 5,
                        actor_id: "merchant",
                        text: line
                    }
                )
                .is_none()
            );
        }
        let mut ambiguous = model.clone();
        let mut other = ambiguous
            .offers
            .iter()
            .find(|offer| offer.id == "clothes-fine")
            .unwrap()
            .clone();
        other.id = "another-clothes-offer".into();
        ambiguous.offers.push(other);
        assert!(
            select(
                &ambiguous,
                Dialogue {
                    sequence: 6,
                    actor_id: "merchant",
                    text: "Fine clothes."
                }
            )
            .is_none()
        );
    }

    #[test]
    fn committed_stock_dialogue_prefers_the_public_current_held_item_and_keeps_custody() {
        let mut model = shop();
        model.people[0].id = "merchant".into();
        for offer in &mut model.offers {
            offer.seller_id = "merchant".into();
        }
        let offer = model
            .offers
            .iter()
            .find(|offer| offer.id == "clothes-fine")
            .unwrap()
            .clone();
        let mut held = model.things[0].clone();
        held.id = offer.id.clone();
        held.name = offer.name.clone();
        held.picture_description = offer.description.clone();
        held.aliases = offer.aliases.clone();
        held.position = "in Sigrun's hands".into();
        held.owner_id = Some("merchant".into());
        held.holder_id = Some("sigrun".into());
        held.in_scene = true;
        held.publicly_visible = true;
        model.things.push(held);

        let focus = select(
            &model,
            Dialogue {
                sequence: 43,
                actor_id: "merchant",
                text: "The fine clothes are fifteen hundred copper.",
            },
        )
        .unwrap();
        assert!(matches!(
            &focus.subject,
            Subject::Thing {
                id,
                owner_id: Some(owner),
                holder_id: Some(holder),
                position,
                ..
            } if id == "clothes-fine"
                && owner == "merchant"
                && holder == "sigrun"
                && position == "in Sigrun's hands"
        ));
        assert!(focus.subject.remains_current(&model));
        assert!(
            model.offer_available("clothes-fine"),
            "mentioning held goods must not imply a sale of current stock"
        );

        let mut private = model.clone();
        private
            .things
            .iter_mut()
            .find(|thing| thing.id == "clothes-fine")
            .unwrap()
            .publicly_visible = false;
        assert!(!focus.subject.remains_current(&private));
        assert!(matches!(
            select(
                &private,
                Dialogue {
                    sequence: 44,
                    actor_id: "merchant",
                    text: "The fine clothes are fifteen hundred copper.",
                }
            )
            .unwrap()
            .subject,
            Subject::Offer { .. }
        ));
    }

    #[test]
    fn reveal_uses_only_canonical_goods_and_excludes_the_hypothetical_from_the_prompt() {
        let model = shop();
        let focus = select(
            &model,
            Dialogue {
                sequence: 42,
                actor_id: "merchant",
                text: "What if the fine clothes burned tomorrow?",
            },
        )
        .unwrap();
        let prompt = crate::gameplay::imagery::narrative_focus::reveal(
            crate::gameplay::startup_configuration::ContentRating::Mature,
            &model,
            &focus,
        )
        .unwrap();
        assert!(prompt.as_str().contains("clothes-fine"));
        assert!(prompt.as_str().contains("1500"));
        assert!(prompt.as_str().contains("canonical room photograph"));
        assert!(prompt.as_str().contains("no ownership transfer"));
        assert!(!prompt.as_str().contains("burned tomorrow"));
        let mut sold = model.clone();
        sold.offers.retain(|offer| offer.id != "clothes-fine");
        assert!(!focus.subject.remains_current(&sold));
        assert!(
            crate::gameplay::imagery::narrative_focus::reveal(
                crate::gameplay::startup_configuration::ContentRating::Mature,
                &sold,
                &focus,
            )
            .is_err()
        );
    }
}
