//! The camera receives canonical IDs selected by committed exchanges, never player words.

use super::*;
use crate::gameplay::adventure::scene_model::{SceneModel, SceneNow};
use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Selection {
    pub(super) scene: String,
    pub(super) hero: String,
    pub(super) target: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct Frame {
    asking_hero: String,
    subject_id: String,
    subject_kind: &'static str,
    name: String,
    description: String,
    position: String,
    holder_id: Option<String>,
    owner_id: Option<String>,
    composition: &'static str,
}

impl Frame {
    pub(super) fn record_request(&self, revision: u64, image_key: &ImageKey) {
        crate::gameplay::actor::record_audit_diagnostic(
            "gameplay.living_scene.subject_requested",
            format!(
                "PLATE subject kind={} id={} asking_hero={} revision={revision} image_key={}",
                self.subject_kind,
                self.subject_id,
                self.asking_hero,
                image_key.as_str()
            ),
            serde_json::Map::from_iter([
                ("subject_kind".into(), serde_json::json!(self.subject_kind)),
                ("subject_id".into(), serde_json::json!(self.subject_id)),
                ("asking_hero".into(), serde_json::json!(self.asking_hero)),
                ("graph_revision".into(), serde_json::json!(revision)),
                ("image_key".into(), serde_json::json!(image_key.as_str())),
            ]),
            &[],
        );
    }
}

impl Selection {
    pub(super) fn narrative(
        &self,
        model: &SceneModel,
        sequence: u64,
        actor_id: &str,
    ) -> Option<crate::gameplay::narrative_focus::NarrativeFocus> {
        use crate::gameplay::narrative_focus::{NarrativeFocus, Subject};
        let subject = if let Some(thing) = model
            .things
            .iter()
            .find(|thing| thing.id == self.target && thing.in_scene && thing.publicly_visible)
        {
            Subject::Thing {
                id: thing.id.clone(),
                name: thing.name.clone(),
                description: thing.picture_description.clone(),
                position: thing.position.clone(),
                owner_id: thing.owner_id.clone(),
                holder_id: thing.holder_id.clone(),
            }
        } else {
            let offer = model.offers.iter().find(|offer| {
                (offer.id == self.target || offer.item_id == self.target)
                    && model.offer_available(&offer.id)
            })?;
            Subject::Offer {
                id: offer.id.clone(),
                name: offer.name.clone(),
                description: offer.description.clone(),
                seller_id: offer.seller_id.clone(),
                cost_cp: offer.cost_cp,
            }
        };
        Some(NarrativeFocus {
            dialogue_sequence: sequence,
            speaker_actor_id: actor_id.to_owned(),
            subject,
        })
    }

    pub(super) fn resolve(
        &self,
        model: &SceneModel,
        cast: &[pack::SceneCastMember],
    ) -> Option<Frame> {
        let target = self.target.strip_prefix("npc:").unwrap_or(&self.target);
        let (kind, name, description, position, holder_id, owner_id) = if let Some(thing) = model
            .things
            .iter()
            .find(|thing| thing.id == target && thing.in_scene && thing.publicly_visible)
        {
            (
                "thing",
                &thing.name,
                &thing.picture_description,
                thing.position.clone(),
                thing.holder_id.clone(),
                thing.owner_id.clone(),
            )
        } else if let Some(offer) = model.offers.iter().find(|offer| {
            (offer.id == target || offer.item_id == target) && model.offer_available(&offer.id)
        }) {
            (
                "offer",
                &offer.name,
                &offer.description,
                String::new(),
                Some(offer.seller_id.clone()),
                Some(offer.seller_id.clone()),
            )
        } else if let Some(person) = model.people.iter().find(|person| person.id == target) {
            (
                "person",
                &person.name,
                &person.description,
                person.position.clone(),
                None,
                None,
            )
        } else if let Some(person) = cast.iter().find(|person| person.id == target) {
            // Public entrants can precede the authored model. Their exact identity is already
            // on the cast sheet; their current position and mood come from the scene graph.
            return Some(Frame {
                asking_hero: self.hero.clone(),
                subject_id: target.to_owned(),
                subject_kind: "person",
                name: person.name.clone(),
                description: String::new(),
                position: String::new(),
                holder_id: None,
                owner_id: None,
                composition: "Frame the addressed person from the canonical cast sheet and include the asking hero; use the current scene graph's recorded position and mood.",
            });
        } else {
            return None;
        };
        Some(Frame {
            asking_hero: self.hero.clone(),
            subject_id: target.to_owned(),
            subject_kind: kind,
            name: name.clone(),
            description: description.clone(),
            position,
            holder_id,
            owner_id,
            composition: "Frame this committed subject closely and include the asking hero. Keep the item in its recorded holder's hands or at its recorded display; keep the addressed person and their recorded position. No implied transaction or new action.",
        })
    }
}

impl super::super::LivingScenes {
    /// The dialogue owner supplies the exact committed/judged target and authenticated hero.
    pub(in crate::gameplay) fn frame_committed_subject(
        &self,
        scene: &pack::Scene,
        snapshot: &SceneNow,
        hero: &str,
        target: &str,
    ) {
        let Some(plates) = &self.plates else { return };
        let selection = Selection {
            scene: projection_scene_identity(
                scene.key.source_label(),
                scene.source_variant_id.as_deref().unwrap_or_default(),
                &scene.title,
                &scene.asset,
            ),
            hero: hero.to_owned(),
            target: target.to_owned(),
        };
        let mut book = locked(&plates.shared.book);
        if book
            .current
            .as_ref()
            .is_some_and(|current| current.scene_key != selection.scene)
        {
            record(
                "gameplay.living_scene.subject_stale",
                &[("subject_id", target.to_owned())],
            );
            return;
        }
        let current_cast = book
            .current
            .as_ref()
            .and_then(|current| current.scene.as_ref())
            .map_or(scene.cast.as_slice(), |current| current.cast.as_slice());
        if selection.resolve(&snapshot.model, current_cast).is_none() {
            book.state_images.subject_selection = None;
            record(
                "gameplay.living_scene.subject_unavailable",
                &[("subject_id", target.to_owned())],
            );
            drop(book);
            plates.shared.wake();
            return;
        }
        if book.state_images.subject_selection.as_ref() == Some(&selection) {
            return;
        }
        book.state_images.subject_selection = Some(selection);
        record(
            "gameplay.living_scene.subject_selected",
            &[
                ("subject_id", target.to_owned()),
                ("asking_hero", hero.to_owned()),
            ],
        );
        drop(book);
        plates.shared.wake();
    }
}
