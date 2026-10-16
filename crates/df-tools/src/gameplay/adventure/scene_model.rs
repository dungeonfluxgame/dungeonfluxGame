//! The generated location model shared by narration, NPCs, interaction, and imagery.

use super::super::pack::{FindingSkill, SceneKey};
use super::setting::{SceneSetting, SettingKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const SCHEMA: &str =
    include_str!("../../../../../content/adventure-tables/scene-model.schema.json");
const PROMPT: &str = include_str!("../../../../../content/adventure-tables/scene-model.prompt.md");
const MAXIMUM_CHANGES: usize = 64;

/// Detached presentation copies published by the session owner after loading or committing state.
/// These copies never resolve an action or replace the durable scene-change ledger.
static PRESENTATION: std::sync::OnceLock<std::sync::Mutex<BTreeMap<String, SceneNow>>> =
    std::sync::OnceLock::new();

pub(crate) fn publish_scene(place_id: &str, snapshot: SceneNow) {
    let mut scenes = PRESENTATION
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    scenes.insert(place_id.to_owned(), snapshot);
}

pub(crate) fn install_scenes(settings: &super::setting::ChapterSettings) {
    let mut scenes = PRESENTATION
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    scenes.clear();
    for setting in [
        &settings.meeting,
        &settings.confrontation,
        &settings.destination,
    ] {
        if let Some(model) = &setting.model {
            scenes.insert(
                setting.place_id.clone(),
                SceneNow {
                    model: model.clone(),
                    changes: Vec::new(),
                },
            );
        }
    }
}

pub(crate) fn current_scene(scene: &super::super::pack::Scene) -> Option<SceneNow> {
    let setting = scene.setting.as_ref()?;
    let scenes = PRESENTATION
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut current = scenes.get(&setting.place_id).cloned().or_else(|| {
        setting.model.clone().map(|model| SceneNow {
            model,
            changes: Vec::new(),
        })
    })?;
    current.model = current.model.for_scene(scene.key);
    Some(current)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScenePlace {
    Meeting,
    Confrontation,
    Destination,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct SceneModel {
    pub(crate) time_of_day: String,
    pub(crate) weather: String,
    pub(crate) people: Vec<ScenePerson>,
    pub(crate) things: Vec<SceneThing>,
    pub(crate) ways_out: Vec<SceneWayOut>,
    pub(crate) offers: Vec<SceneOffer>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScenePerson {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) role: String,
    /// One line that says how this person looks and what they are doing.
    pub(crate) description: String,
    pub(crate) relation_to_speaker: String,
    pub(crate) position: String,
    pub(crate) aliases: Vec<String>,
    pub(crate) is_background: bool,
    pub(crate) present_in: Vec<String>,
    pub(crate) attitude: StartingAttitude,
    pub(crate) attitudes: BTreeMap<String, StartingAttitude>,
    pub(crate) knowledge: Vec<SceneKnowledge>,
    pub(crate) wants: String,
    /// Private purse used only by server-authorized NPC commerce actions.
    #[serde(default)]
    pub(crate) coin_cp: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct SceneKnowledge {
    pub(crate) fact_id: String,
    pub(crate) statement: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StartingAttitude {
    Friendly,
    Indifferent,
    Hostile,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct SceneThing {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) short: String,
    pub(crate) detail: String,
    pub(crate) picture_description: String,
    pub(crate) closer_look: String,
    pub(crate) skill: FindingSkill,
    pub(crate) dc: u8,
    pub(crate) finding_id: Option<String>,
    pub(crate) clue_fact_id: Option<String>,
    pub(crate) portable: bool,
    pub(crate) value_cp: Option<u32>,
    pub(crate) use_effect: String,
    pub(crate) position: String,
    pub(crate) owner_id: Option<String>,
    pub(crate) in_use: bool,
    pub(crate) in_scene: bool,
    /// Explicit consent that a held object may appear on the shared display.
    /// Old authored scenes default to visible so existing public props retain behavior.
    #[serde(default = "default_shared_visible")]
    pub(crate) publicly_visible: bool,
    pub(crate) holder_id: Option<String>,
    pub(crate) open: bool,
    pub(crate) lit: bool,
    pub(crate) broken: bool,
    pub(crate) searched_by: Vec<String>,
    pub(crate) aliases: Vec<String>,
    /// Stable provenance for the item instance. Ownership and physical custody are
    /// deliberately separate: a scene refresh must never manufacture a second item.
    #[serde(default)]
    pub(crate) provenance: SceneItemProvenance,
}

/// Identifies where an item instance came from without conflating its owner, holder,
/// or current scene position. `source_id` is stable within the generated location pack.
#[derive(Clone, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct SceneItemProvenance {
    pub(crate) kind: String,
    pub(crate) source_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) seller_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SceneInventoryLocation {
    Stock,
    Scene { position: String },
    Held { holder_id: String },
}

/// Audience-safe inventory projection. It contains stable item identity and public
/// placement/price state, while private clues and NPC knowledge remain on SceneThing.
#[derive(Clone, Debug, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct SceneInventoryEntry {
    pub(crate) item_id: String,
    pub(crate) source_id: String,
    pub(crate) provenance_kind: String,
    pub(crate) owner_id: Option<String>,
    pub(crate) holder_id: Option<String>,
    pub(crate) location: SceneInventoryLocation,
    pub(crate) available: bool,
    pub(crate) cost_cp: Option<u32>,
}

fn default_shared_visible() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct SceneWayOut {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) aliases: Vec<String>,
    pub(crate) position: String,
    pub(crate) beyond_place_id: String,
    pub(crate) beyond: String,
    pub(crate) open: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct SceneOffer {
    pub(crate) id: String,
    /// The concrete item instance represented by this finite stock offer. It exists
    /// before purchase in the inventory projection and is never replaced by a clone.
    pub(crate) item_id: String,
    pub(crate) seller_id: String,
    pub(crate) name: String,
    pub(crate) aliases: Vec<String>,
    pub(crate) description: String,
    /// Exact copper-piece price from the SRD price table.
    pub(crate) cost_cp: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SceneTargetKind {
    Person,
    Thing,
    WayOut,
    Offer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedTarget {
    pub(crate) kind: SceneTargetKind,
    /// A pointing phrase can resolve to a group of people; actions still address a stable ID.
    pub(crate) ids: Vec<String>,
    pub(crate) name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct SceneChange {
    pub(crate) entity_id: String,
    pub(crate) description: String,
    pub(crate) kind: SceneChangeKind,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum SceneChangeKind {
    /// Converts one finite catalog offer into a durable, held scene thing.
    /// The offer id remains the ledger key so replay is deterministic.
    Purchased {
        holder_id: String,
        seller_id: String,
    },
    NpcPurchased {
        holder_id: String,
        seller_id: String,
        cost_cp: u32,
    },
    Taken {
        holder_id: String,
    },
    /// Custody changes without transferring ownership. The owner can react through
    /// ordinary NPC interaction once the theft is witnessed or discovered.
    Stolen {
        holder_id: String,
    },
    Given {
        holder_id: String,
        /// Present only when the giver is the current owner. A borrowed observation
        /// changes hands without minting ownership for the recipient.
        #[serde(default)]
        owner_id: Option<String>,
    },
    Returned {
        position: String,
    },
    Opened,
    Closed,
    Lit,
    Extinguished,
    Broken,
    Moved {
        position: String,
    },
    Searched {
        hero_id: String,
    },
    PersonMoved {
        position: String,
    },
    AttitudeChanged {
        hero_id: String,
        attitude: StartingAttitude,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct SceneNow {
    pub(crate) model: SceneModel,
    pub(crate) changes: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SceneModelError {
    MissingFact,
    InvalidShape,
    InvalidBounds,
    DuplicateId,
    UnknownPerson,
    UnknownPlace,
    UnknownFact,
    UnknownFinding,
    Contradiction,
    UnknownEntity,
    TooManyChanges,
    Serialization,
}

impl std::fmt::Display for SceneModelError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::MissingFact => "scene model needs a generated fact",
            Self::InvalidShape => "scene model has an invalid structured shape",
            Self::InvalidBounds => "scene model exceeded its content bounds",
            Self::DuplicateId => "scene model repeats an entity ID",
            Self::UnknownPerson => "scene model names a person outside the adventure cast",
            Self::UnknownPlace => "scene model names a place outside the adventure",
            Self::UnknownFact => "scene model cites a fact outside the adventure",
            Self::UnknownFinding => "scene model cites a finding outside the scene",
            Self::Contradiction => "scene model contradicts a supplied adventure fact",
            Self::UnknownEntity => "scene change names an entity outside the scene model",
            Self::TooManyChanges => "scene change history exceeded its bound",
            Self::Serialization => "scene model serialization failed",
        })
    }
}

impl std::error::Error for SceneModelError {}

impl SceneModel {
    /// Finite catalog stock survives in authoring for replay/refusal; a committed purchase
    /// consumes that offer by creating its canonical owned thing.
    pub(crate) fn offer_available(&self, id: &str) -> bool {
        self.offers.iter().any(|offer| offer.id == id)
            && !self.things.iter().any(|thing| {
                self.offers
                    .iter()
                    .find(|offer| offer.id == id)
                    .is_some_and(|offer| thing.id == offer.item_id)
            })
    }

    /// Returns the canonical, audience-safe inventory for this location. Offers are
    /// represented as stock until purchased; a purchase replaces that stock entry with
    /// the one durable `SceneThing` carrying the same source ID. This is intentionally a
    /// projection of the scene model, so reconnects and image plate rebuilds cannot create
    /// additional item instances.
    pub(crate) fn inventory(&self) -> Vec<SceneInventoryEntry> {
        let mut entries = self
            .things
            .iter()
            .filter(|thing| thing.in_scene || thing.holder_id.is_some())
            .map(|thing| {
                let source_id = if thing.provenance.source_id.is_empty() {
                    thing.id.clone()
                } else {
                    thing.provenance.source_id.clone()
                };
                SceneInventoryEntry {
                    item_id: thing.id.clone(),
                    source_id,
                    provenance_kind: if thing.provenance.kind.is_empty() {
                        "authored_location".into()
                    } else {
                        thing.provenance.kind.clone()
                    },
                    owner_id: thing.owner_id.clone(),
                    holder_id: thing.holder_id.clone(),
                    location: thing.holder_id.as_ref().map_or_else(
                        || SceneInventoryLocation::Scene {
                            position: thing.position.clone(),
                        },
                        |holder_id| SceneInventoryLocation::Held {
                            holder_id: holder_id.clone(),
                        },
                    ),
                    available: thing.holder_id.is_none() && thing.in_scene && thing.portable,
                    cost_cp: thing.value_cp,
                }
            })
            .collect::<Vec<_>>();
        entries.extend(
            self.offers
                .iter()
                .filter(|offer| self.offer_available(&offer.id))
                .map(|offer| SceneInventoryEntry {
                    item_id: offer.item_id.clone(),
                    source_id: offer.id.clone(),
                    provenance_kind: "generated_offer".into(),
                    owner_id: Some(offer.seller_id.clone()),
                    holder_id: None,
                    location: SceneInventoryLocation::Stock,
                    available: true,
                    cost_cp: Some(offer.cost_cp),
                }),
        );
        entries.sort_by(|left, right| left.item_id.cmp(&right.item_id));
        entries
    }

    pub(crate) fn inventory_json(&self) -> Result<Value, SceneModelError> {
        serde_json::to_value(self.inventory()).map_err(|_| SceneModelError::Serialization)
    }

    pub(crate) fn public_contents(&self) -> Result<String, SceneModelError> {
        let mut contents: Value = serde_json::from_str(&self.picture_state()?)
            .map_err(|_| SceneModelError::Serialization)?;
        if let Some(things) = contents.get_mut("things").and_then(Value::as_array_mut) {
            for (view, thing) in things.iter_mut().zip(&self.things) {
                view["short"] = Value::String(thing.short.clone());
                view["actions"] = serde_json::json!(if thing.portable {
                    vec!["look", "take", "steal", "use"]
                } else {
                    vec!["look", "use"]
                });
                view["owner"] = serde_json::json!(thing.owner_id);
            }
        }
        contents["offers"] = serde_json::json!(
            self.offers
                .iter()
                .filter(|offer| self.offer_available(&offer.id))
                .map(|offer| {
                    let mut value = serde_json::json!(offer);
                    value["actions"] = serde_json::json!(["look", "buy"]);
                    value
                })
                .collect::<Vec<_>>()
        );
        contents["inventory"] = self.inventory_json()?;
        serde_json::to_string(&contents).map_err(|_| SceneModelError::Serialization)
    }

    /// Visible material only: private knowledge, undiscovered clues and searches never reach art.
    pub(crate) fn picture_state(&self) -> Result<String, SceneModelError> {
        let people = self
            .people
            .iter()
            .map(|person| {
                serde_json::json!({
                    "id": person.id, "name": person.name, "look_and_doing": person.description,
                    "position": person.position,
                })
            })
            .collect::<Vec<_>>();
        let things = self
            .things
            .iter()
            .map(|thing| {
                serde_json::json!({
                    "id": thing.id, "name": thing.name, "look": thing.picture_description,
                    "position": thing.position, "in_scene": thing.in_scene,
                    "publicly_visible": thing.publicly_visible,
                    "holder": thing.holder_id, "open": thing.open, "lit": thing.lit,
                    "broken": thing.broken,
                })
            })
            .collect::<Vec<_>>();
        let ways_out = self
            .ways_out
            .iter()
            .map(|way| {
                serde_json::json!({
                    "id": way.id, "name": way.name, "position": way.position,
                    "open": way.open, "beyond": way.beyond,
                })
            })
            .collect::<Vec<_>>();
        serde_json::to_string(&serde_json::json!({
            "time_of_day": self.time_of_day, "weather": self.weather,
            "people": people, "things": things, "ways_out": ways_out,
        }))
        .map_err(|_| SceneModelError::Serialization)
    }

    /// Table fallback is generated before prose so the same room facts guide narration and art.
    pub(crate) fn fallback(
        setting: &SceneSetting,
        place: ScenePlace,
        seed: u64,
        facts: &BTreeMap<String, String>,
    ) -> Result<Self, SceneModelError> {
        let place_prefix = place.prefix();
        let place_fact_id = format!("setting.{place_prefix}.short");
        let stake = required(facts, "stake.short")?;
        let mut people = named_people(setting, place, facts, &place_fact_id, stake)?;
        let backgrounds = background_people(setting, place, seed, facts, &place_fact_id)?;
        people.extend(backgrounds);

        let (time_of_day, weather) = scene_conditions(setting, seed);
        let things = things(setting, place, &people)?;
        let ways_out = ways_out(setting, &time_of_day, &weather);
        let offers = offers(setting, &people);
        let model = Self {
            time_of_day,
            weather,
            people,
            things,
            ways_out,
            offers,
        };
        model.validate_structure()?;
        Ok(model)
    }

    pub(crate) fn prompt() -> &'static str {
        PROMPT
    }

    pub(crate) fn schema() -> Result<Value, SceneModelError> {
        serde_json::from_str(SCHEMA).map_err(|_| SceneModelError::InvalidShape)
    }

    pub(crate) fn validate_structure(&self) -> Result<(), SceneModelError> {
        if !(2..=6).contains(&self.people.len())
            || !(4..=6).contains(&self.things.len())
            || !(1..=3).contains(&self.ways_out.len())
            || self.offers.len() > 8
            || self.time_of_day.trim().is_empty()
            || self.weather.trim().is_empty()
            || self.time_of_day.chars().count() > 32
            || self.weather.chars().count() > 72
        {
            return Err(SceneModelError::InvalidBounds);
        }
        let mut ids = BTreeSet::new();
        for person in &self.people {
            if !ids.insert(person.id.as_str()) {
                return Err(SceneModelError::DuplicateId);
            }
            if !valid_text(&person.id, 96)
                || !valid_text(&person.name, 64)
                || !valid_text(&person.role, 48)
                || !valid_text(&person.description, 180)
                || !valid_text(&person.relation_to_speaker, 100)
                || !valid_text(&person.position, 48)
                || !valid_text(&person.wants, 100)
                || person.aliases.is_empty()
                || person.aliases.len() > 8
                || person.present_in.is_empty()
                || person.present_in.len() > 5
                || person.knowledge.len() != 3
                || person.knowledge.iter().any(|knowledge| {
                    !valid_text(&knowledge.fact_id, 96) || !valid_text(&knowledge.statement, 160)
                })
                || person.aliases.iter().any(|alias| !valid_text(alias, 64))
            {
                return Err(SceneModelError::InvalidBounds);
            }
        }
        for thing in &self.things {
            if !ids.insert(thing.id.as_str()) {
                return Err(SceneModelError::DuplicateId);
            }
            if !valid_text(&thing.id, 96)
                || !valid_text(&thing.name, 64)
                || !valid_text(&thing.short, 96)
                || !valid_text(&thing.detail, 480)
                || !valid_text(&thing.picture_description, 120)
                || !valid_text(&thing.closer_look, 180)
                || !valid_text(&thing.use_effect, 120)
                || !valid_text(&thing.position, 48)
                || !(5..=30).contains(&thing.dc)
                || thing.aliases.is_empty()
                || thing.aliases.len() > 8
                || thing.aliases.iter().any(|alias| !valid_text(alias, 64))
                || !(2..=4).contains(&sentence_count(&thing.detail))
                || thing
                    .owner_id
                    .as_ref()
                    .is_some_and(|id| !ids.contains(id.as_str()))
                || (!thing.provenance.kind.is_empty()
                    && (!valid_text(&thing.provenance.kind, 48)
                        || !valid_text(&thing.provenance.source_id, 96)
                        || thing
                            .provenance
                            .seller_id
                            .as_ref()
                            .is_some_and(|id| !ids.contains(id.as_str()))))
            {
                return Err(SceneModelError::InvalidBounds);
            }
        }
        for way_out in &self.ways_out {
            if !ids.insert(way_out.id.as_str()) {
                return Err(SceneModelError::DuplicateId);
            }
            if !valid_text(&way_out.id, 96)
                || !valid_text(&way_out.name, 64)
                || !valid_text(&way_out.position, 48)
                || !valid_text(&way_out.beyond_place_id, 96)
                || !valid_text(&way_out.beyond, 180)
                || way_out.aliases.is_empty()
                || way_out.aliases.len() > 6
            {
                return Err(SceneModelError::InvalidBounds);
            }
        }
        for offer in &self.offers {
            if !ids.insert(offer.id.as_str()) {
                return Err(SceneModelError::DuplicateId);
            }
            // Offer handles and concrete item identities use separate namespaces in
            // authoring, but both must be unique in the canonical scene graph. A purchase
            // promotes this item ID into `things`; rejecting collisions here prevents a
            // second stock entry or an authored object from shadowing that identity.
            if !ids.insert(offer.item_id.as_str()) {
                return Err(SceneModelError::DuplicateId);
            }
            if !valid_text(&offer.id, 96)
                || !valid_text(&offer.item_id, 96)
                || !valid_text(&offer.seller_id, 96)
                || !valid_text(&offer.name, 64)
                || !valid_text(&offer.description, 120)
                || offer.aliases.is_empty()
                || offer.aliases.len() > 6
            {
                return Err(SceneModelError::InvalidBounds);
            }
        }
        let person_ids = self
            .people
            .iter()
            .map(|person| person.id.as_str())
            .collect::<BTreeSet<_>>();
        if self
            .things
            .iter()
            .filter_map(|thing| thing.owner_id.as_deref())
            .any(|owner| !person_ids.contains(owner))
            || self
                .offers
                .iter()
                .any(|offer| !person_ids.contains(offer.seller_id.as_str()))
        {
            return Err(SceneModelError::UnknownPerson);
        }
        Ok(())
    }

    /// Rejects structured output whose cast, place, finding, or cited knowledge is not grounded.
    pub(crate) fn validate_candidate(
        value: Value,
        cast: &BTreeMap<String, String>,
        place_ids: &BTreeSet<String>,
        canonical_facts: &BTreeMap<String, String>,
        finding_ids: &BTreeSet<String>,
    ) -> Result<Self, SceneModelError> {
        let mut model: Self =
            serde_json::from_value(value).map_err(|_| SceneModelError::InvalidShape)?;
        for thing in &mut model.things {
            if thing.provenance.kind.is_empty() {
                thing.provenance.kind = "authored_location".into();
            }
            if thing.provenance.source_id.is_empty() {
                thing.provenance.source_id = thing.id.clone();
            }
        }
        model.validate_structure()?;
        for person in &model.people {
            if !person.is_background && cast.get(&person.id).is_none_or(|name| name != &person.name)
            {
                return Err(SceneModelError::UnknownPerson);
            }
            for knowledge in &person.knowledge {
                let expected = if knowledge.fact_id == format!("scene-person:{}", person.id) {
                    Some(format!("{} is a {}", person.name, person.role))
                } else {
                    canonical_facts.get(&knowledge.fact_id).cloned()
                };
                let Some(expected) = expected else {
                    return Err(SceneModelError::UnknownFact);
                };
                if knowledge.statement != expected {
                    return Err(SceneModelError::Contradiction);
                }
            }
        }
        for thing in &model.things {
            if thing
                .clue_fact_id
                .as_ref()
                .is_some_and(|id| !canonical_facts.contains_key(id))
            {
                return Err(SceneModelError::UnknownFact);
            }
            if thing
                .finding_id
                .as_ref()
                .is_some_and(|id| !finding_ids.contains(id))
            {
                return Err(SceneModelError::UnknownFinding);
            }
        }
        if model.ways_out.iter().any(|way_out| {
            way_out.beyond_place_id != "none" && !place_ids.contains(&way_out.beyond_place_id)
        }) {
            return Err(SceneModelError::UnknownPlace);
        }
        Ok(model)
    }

    pub(crate) fn for_scene(&self, scene_key: SceneKey) -> Self {
        let key = scene_key_label(scene_key);
        let mut model = self.clone();
        model
            .people
            .retain(|person| person.present_in.iter().any(|scene| scene == key));
        model
    }

    /// The single prompt projection for the immutable model plus every accepted state change.
    pub(crate) fn as_it_is_now(
        &self,
        scene_key: SceneKey,
        changes: &[SceneChange],
    ) -> Result<String, SceneModelError> {
        if changes.len() > MAXIMUM_CHANGES {
            return Err(SceneModelError::TooManyChanges);
        }
        let mut current = self.clone();
        let mut descriptions = Vec::with_capacity(changes.len());
        for change in changes {
            if !valid_text(&change.description, 180) {
                return Err(SceneModelError::InvalidBounds);
            }
            current.apply(change)?;
            descriptions.push(change.description.clone());
        }
        let snapshot = SceneNow {
            model: current.for_scene(scene_key),
            changes: descriptions,
        };
        serde_json::to_string(&snapshot).map_err(|_| SceneModelError::Serialization)
    }

    pub(crate) fn resolve_target(&self, line: &str) -> Option<ResolvedTarget> {
        let line = normalize(line);
        if line.is_empty() {
            return None;
        }
        let object_action = [
            "take", "pick up", "examine", "inspect", "look at", "use", "open", "read", "sit",
            "warm", "return", "put", "drop", "give",
        ]
        .iter()
        .any(|action| contains_phrase(&line, action));
        if object_action {
            if let Some(thing) = best_visible_thing(&self.things, &line) {
                return Some(ResolvedTarget {
                    kind: SceneTargetKind::Thing,
                    ids: vec![thing.id.clone()],
                    name: thing.name.clone(),
                });
            }
            if let Some(way_out) = best_match(&self.ways_out, &line, |way_out| &way_out.aliases) {
                return Some(ResolvedTarget {
                    kind: SceneTargetKind::WayOut,
                    ids: vec![way_out.id.clone()],
                    name: way_out.name.clone(),
                });
            }
        }
        if let Some(offer) = best_match(&self.offers, &line, |offer| &offer.aliases) {
            return Some(ResolvedTarget {
                kind: SceneTargetKind::Offer,
                ids: vec![offer.id.clone()],
                name: offer.name.clone(),
            });
        }
        let people = matching_people(&self.people, &line);
        if !people.is_empty() {
            let ids = people
                .iter()
                .map(|person| person.id.clone())
                .collect::<Vec<_>>();
            let name = if people.len() == 1 {
                people[0].name.clone()
            } else {
                people
                    .iter()
                    .map(|person| person.name.as_str())
                    .collect::<Vec<_>>()
                    .join(" and ")
            };
            return Some(ResolvedTarget {
                kind: SceneTargetKind::Person,
                ids,
                name,
            });
        }
        if let Some(thing) = best_visible_thing(&self.things, &line) {
            return Some(ResolvedTarget {
                kind: SceneTargetKind::Thing,
                ids: vec![thing.id.clone()],
                name: thing.name.clone(),
            });
        }
        best_match(&self.ways_out, &line, |way_out| &way_out.aliases).map(|way_out| {
            ResolvedTarget {
                kind: SceneTargetKind::WayOut,
                ids: vec![way_out.id.clone()],
                name: way_out.name.clone(),
            }
        })
    }

    fn apply(&mut self, change: &SceneChange) -> Result<(), SceneModelError> {
        let person_ids = self
            .people
            .iter()
            .map(|person| person.id.as_str())
            .collect::<BTreeSet<_>>();
        if let Some(thing) = self
            .things
            .iter_mut()
            .find(|thing| thing.id == change.entity_id)
        {
            match &change.kind {
                SceneChangeKind::Purchased { .. } | SceneChangeKind::NpcPurchased { .. } => {
                    return Err(SceneModelError::Contradiction);
                }
                SceneChangeKind::Taken { holder_id }
                | SceneChangeKind::Stolen { holder_id } => {
                    thing.holder_id = Some(holder_id.clone());
                    thing.in_scene = person_ids.contains(holder_id.as_str());
                    thing.position = format!("held by {holder_id}");
                }
                SceneChangeKind::Given {
                    holder_id,
                    owner_id,
                } => {
                    thing.holder_id = Some(holder_id.clone());
                    if let Some(owner_id) = owner_id {
                        thing.owner_id = Some(owner_id.clone());
                    }
                    thing.in_scene = person_ids.contains(holder_id.as_str());
                    thing.position = format!("held by {holder_id}");
                }
                SceneChangeKind::Returned { position } => {
                    thing.holder_id = None;
                    thing.in_scene = true;
                    thing.in_use = false;
                    thing.position = position.clone();
                }
                SceneChangeKind::Opened => thing.open = true,
                SceneChangeKind::Closed => thing.open = false,
                SceneChangeKind::Lit => thing.lit = true,
                SceneChangeKind::Extinguished => thing.lit = false,
                SceneChangeKind::Broken => thing.broken = true,
                SceneChangeKind::Moved { position } => thing.position = position.clone(),
                SceneChangeKind::Searched { hero_id } => {
                    if !thing.searched_by.contains(hero_id) {
                        thing.searched_by.push(hero_id.clone());
                    }
                }
                SceneChangeKind::PersonMoved { .. } | SceneChangeKind::AttitudeChanged { .. } => {
                    return Err(SceneModelError::UnknownEntity);
                }
            }
            return Ok(());
        }
        let purchase = match &change.kind {
            SceneChangeKind::Purchased {
                holder_id,
                seller_id,
            } => Some((holder_id, seller_id, None)),
            SceneChangeKind::NpcPurchased {
                holder_id,
                seller_id,
                cost_cp,
            } => Some((holder_id, seller_id, Some(*cost_cp))),
            _ => None,
        };
        if let Some((holder_id, _seller_id, npc_cost)) = purchase {
            let offer = self
                .offers
                .iter()
                .find(|offer| offer.id == change.entity_id)
                .ok_or(SceneModelError::UnknownEntity)?;
            let item_id = offer.item_id.clone();
            if self.things.iter().any(|thing| thing.id == item_id) {
                return Err(SceneModelError::Contradiction);
            }
            let seller_name = self
                .people
                .iter()
                .find(|person| person.id.as_str() == _seller_id.as_str())
                .map(|person| person.name.as_str())
                .unwrap_or("the seller");
            if let Some(cost_cp) = npc_cost {
                if cost_cp != offer.cost_cp {
                    return Err(SceneModelError::Contradiction);
                }
                let Some(payer) = self
                    .people
                    .iter_mut()
                    .find(|person| person.id == *holder_id)
                else {
                    return Err(SceneModelError::UnknownPerson);
                };
                payer.coin_cp = payer
                    .coin_cp
                    .checked_sub(cost_cp)
                    .ok_or(SceneModelError::Contradiction)?;
            }
            let detail = format!(
                "{}. It was bought from {}.",
                offer.description.trim_end_matches('.'),
                seller_name
            );
            self.things.push(SceneThing {
                id: item_id,
                name: offer.name.clone(),
                short: offer.name.clone(),
                detail: detail.clone(),
                picture_description: offer.description.clone(),
                closer_look: detail,
                skill: FindingSkill::Investigation,
                // Purchased goods are not findings, but the durable SceneThing schema still
                // requires its bounded investigation DC. Keep the neutral catalog value within
                // that schema range so ownership validation can admit the item.
                dc: 10,
                finding_id: None,
                clue_fact_id: None,
                portable: true,
                value_cp: Some(offer.cost_cp),
                use_effect: format!("Uses {}.", offer.description.trim_end_matches('.')),
                position: format!("held by {holder_id}"),
                // Ownership transfers to the buyer; seller_id remains in the
                // durable purchase change as provenance for the transaction.
                owner_id: Some(holder_id.clone()),
                in_use: false,
                in_scene: true,
                publicly_visible: true,
                holder_id: Some(holder_id.clone()),
                open: false,
                lit: false,
                broken: false,
                searched_by: Vec::new(),
                aliases: offer.aliases.clone(),
                provenance: SceneItemProvenance {
                    kind: "generated_offer".into(),
                    source_id: offer.id.clone(),
                    seller_id: Some(_seller_id.clone()),
                },
            });
            return Ok(());
        }
        if let Some(person) = self
            .people
            .iter_mut()
            .find(|person| person.id == change.entity_id)
        {
            match &change.kind {
                SceneChangeKind::PersonMoved { position } => person.position = position.clone(),
                SceneChangeKind::AttitudeChanged { hero_id, attitude } => {
                    person.attitudes.insert(hero_id.clone(), *attitude);
                }
                _ => return Err(SceneModelError::UnknownEntity),
            }
            return Ok(());
        }
        if let Some(way_out) = self
            .ways_out
            .iter_mut()
            .find(|way_out| way_out.id == change.entity_id)
        {
            match change.kind {
                SceneChangeKind::Opened => way_out.open = true,
                SceneChangeKind::Closed => way_out.open = false,
                _ => return Err(SceneModelError::UnknownEntity),
            }
            return Ok(());
        }
        Err(SceneModelError::UnknownEntity)
    }
}

impl ScenePlace {
    fn prefix(self) -> &'static str {
        match self {
            Self::Meeting => "meeting",
            Self::Confrontation => "confrontation",
            Self::Destination => "destination",
        }
    }

    fn scene_keys(self) -> &'static [&'static str] {
        match self {
            Self::Meeting => &["opening", "dialogue_seal", "dialogue_escort"],
            Self::Confrontation => &["room", "combat", "defeat"],
            Self::Destination => &["victory", "inn"],
        }
    }
}

pub(crate) fn prompt_context(
    model: &SceneModel,
    scene_key: SceneKey,
) -> Result<String, SceneModelError> {
    serde_json::to_string(&model.for_scene(scene_key)).map_err(|_| SceneModelError::Serialization)
}

fn named_people(
    setting: &SceneSetting,
    place: ScenePlace,
    facts: &BTreeMap<String, String>,
    place_fact_id: &str,
    stake: &str,
) -> Result<Vec<ScenePerson>, SceneModelError> {
    let mut people = Vec::new();
    let mut add = |id: &str,
                   prefix: &str,
                   name_key: &str,
                   role_key: &str,
                   description_key: &str,
                   position: &str,
                   aliases: Vec<String>,
                   present_in: Vec<String>|
     -> Result<(), SceneModelError> {
        let name = required(facts, name_key)?.to_owned();
        let role = required(facts, role_key)?.to_owned();
        let appearance = required(facts, description_key)?;
        let self_fact = format!("{name} is a {role}");
        let relation_to_speaker = match (place, prefix) {
            (ScenePlace::Meeting, "patron") => "the person who hired the party".to_owned(),
            (ScenePlace::Meeting, "ant") => "the danger arriving in the meeting place".to_owned(),
            (ScenePlace::Confrontation, "ant") => "the enemy tied to the errand".to_owned(),
            (ScenePlace::Destination, "patron") => "the party's patron".to_owned(),
            (ScenePlace::Destination, "contact") => {
                "the person the patron sent the party to meet".to_owned()
            }
            _ => "a named person in the adventure".to_owned(),
        };
        let mut all_aliases = vec![name.clone(), role.clone(), position.to_owned()];
        if setting.kind == SettingKind::Shop && prefix == "patron" {
            all_aliases.push("merchant".to_owned());
        }
        all_aliases.extend(aliases);
        people.push(ScenePerson {
            id: id.to_owned(),
            name,
            role,
            description: format!("{appearance}; {position}."),
            relation_to_speaker,
            position: position.to_owned(),
            aliases: all_aliases,
            is_background: false,
            present_in,
            attitude: StartingAttitude::Indifferent,
            attitudes: BTreeMap::new(),
            knowledge: vec![
                SceneKnowledge {
                    fact_id: place_fact_id.to_owned(),
                    statement: setting.short.clone(),
                },
                SceneKnowledge {
                    fact_id: "stake.short".to_owned(),
                    statement: stake.to_owned(),
                },
                SceneKnowledge {
                    fact_id: format!("scene-person:{id}"),
                    statement: self_fact,
                },
            ],
            wants: format!("wants the party's errand concerning {stake} to reach a clear end"),
            coin_cp: if setting.kind == SettingKind::Shop {
                5_000
            } else {
                500
            },
        });
        Ok(())
    };

    match place {
        ScenePlace::Meeting => {
            add(
                "patron",
                "patron",
                "patron.full",
                "patron.role",
                "patron.desc",
                "near the heart of the meeting place",
                if setting.kind == SettingKind::Inn {
                    vec!["the host".to_owned(), "the innkeeper".to_owned()]
                } else {
                    vec!["the host".to_owned()]
                },
                place
                    .scene_keys()
                    .iter()
                    .map(|key| (*key).to_owned())
                    .collect(),
            )?;
            // A Shop meeting is a quiet public scene. The antagonist remains authored in the
            // chapter and is projected into the confrontation model, but is not a person in the
            // shop's dialogue phases. Keeping the model projection in lockstep with pack.cast
            // prevents the plate from drawing an uncast creature behind the merchant.
            if setting.kind != SettingKind::Shop {
                add(
                    "antagonist",
                    "ant",
                    "ant.full",
                    "ant.group",
                    "ant.short",
                    "at the room's edge",
                    vec!["the enemy".to_owned(), "the stranger".to_owned()],
                    vec!["dialogue_seal".to_owned(), "dialogue_escort".to_owned()],
                )?;
            }
        }
        ScenePlace::Confrontation => add(
            "antagonist",
            "ant",
            "ant.full",
            "ant.group",
            "ant.short",
            "near the confrontation's center",
            vec!["the enemy".to_owned()],
            vec!["room".to_owned(), "combat".to_owned(), "defeat".to_owned()],
        )?,
        ScenePlace::Destination => {
            add(
                "patron",
                "patron",
                "patron.full",
                "patron.role",
                "patron.desc",
                "beside the arrival point",
                vec!["the host".to_owned()],
                place
                    .scene_keys()
                    .iter()
                    .map(|key| (*key).to_owned())
                    .collect(),
            )?;
            add(
                "contact",
                "contact",
                "contact.full",
                "contact.role",
                "contact.desc",
                "near the destination's entrance",
                vec!["the contact".to_owned()],
                place
                    .scene_keys()
                    .iter()
                    .map(|key| (*key).to_owned())
                    .collect(),
            )?;
        }
    }
    Ok(people)
}

fn background_people(
    setting: &SceneSetting,
    place: ScenePlace,
    seed: u64,
    facts: &BTreeMap<String, String>,
    place_fact_id: &str,
) -> Result<Vec<ScenePerson>, SceneModelError> {
    const NAMES: [&str; 16] = [
        "Mara", "Oren", "Tessa", "Bram", "Nella", "Corin", "Jessa", "Rusk", "Elin", "Perrin",
        "Sella", "Tovin", "Hesta", "Dain", "Lio", "Vera",
    ];
    let roles = background_roles(setting.kind);
    let first = stable_offset(seed, &setting.place_id) % NAMES.len();
    let present_in = place
        .scene_keys()
        .iter()
        .map(|key| (*key).to_owned())
        .collect::<Vec<_>>();
    let mut people = Vec::with_capacity(roles.len());
    for (index, role) in roles.iter().enumerate() {
        let name = NAMES[(first + index) % NAMES.len()].to_owned();
        let (description, position, mut aliases, wants) =
            person_description(setting.kind, index, role);
        aliases.splice(0..0, [name.clone(), role.clone()]);
        aliases.push(position.clone());
        let relation = if setting.kind == SettingKind::Inn && role.as_str() == "serving woman" {
            "works alongside the innkeeper".to_owned()
        } else {
            format!("a local {} at {}", role, setting.name)
        };
        let id = format!("{}-bystander-{}", setting.place_id, index + 1);
        let self_fact = format!("{name} is a {role}");
        let place_value = setting.short.clone();
        let stake_value = required(facts, "stake.short")?.to_owned();
        people.push(ScenePerson {
            id: id.clone(),
            name,
            role: role.clone(),
            description,
            relation_to_speaker: relation,
            position,
            aliases,
            is_background: true,
            present_in: present_in.clone(),
            attitude: StartingAttitude::Indifferent,
            attitudes: BTreeMap::new(),
            knowledge: vec![
                SceneKnowledge {
                    fact_id: place_fact_id.to_owned(),
                    statement: place_value,
                },
                SceneKnowledge {
                    fact_id: "stake.short".to_owned(),
                    statement: stake_value,
                },
                SceneKnowledge {
                    fact_id: format!("scene-person:{id}"),
                    statement: self_fact,
                },
            ],
            wants,
            coin_cp: 500,
        });
    }
    Ok(people)
}

fn background_roles(kind: SettingKind) -> Vec<String> {
    match kind {
        SettingKind::Inn => vec!["carter".into(), "carter".into(), "serving woman".into()],
        SettingKind::TeaHouse | SettingKind::EelCatchersHall | SettingKind::FerrymansHut => {
            vec!["boat hand".into(), "local worker".into()]
        }
        SettingKind::Archive
        | SettingKind::LibraryHall
        | SettingKind::LibraryForecourt
        | SettingKind::ArchivistsStudy
        | SettingKind::ReadingGallery
        | SettingKind::MapRoom => vec!["scribe".into(), "reader".into()],
        SettingKind::Shop => {
            vec!["adventurer".into(), "soldier".into()]
        }
        SettingKind::Apothecary | SettingKind::LanternShop => {
            vec!["shop assistant".into(), "customer".into()]
        }
        SettingKind::Quay | SettingKind::Boathouse | SettingKind::NetLoft => {
            vec!["dock worker".into(), "traveler".into()]
        }
        _ => vec!["local worker".into(), "visitor".into()],
    }
}

fn person_description(
    kind: SettingKind,
    index: usize,
    role: &str,
) -> (String, String, Vec<String>, String) {
    if kind == SettingKind::Inn && role == "carter" {
        return (
            "a broad-shouldered traveler in a road-dusted coat rolls dice with the other carter"
                .into(),
            "at the back table".into(),
            vec![
                "carters".into(),
                "the men at the back table".into(),
                "those men in the back".into(),
                "two men at the back table".into(),
                "the carters".into(),
            ],
            "to finish the dice game before the road calls again".into(),
        );
    }
    if kind == SettingKind::Inn && role == "serving woman" {
        return (
            "a woman in a linen apron carries a tray of mugs".into(),
            "by the bar on the left".into(),
            vec![
                "lady with the tray".into(),
                "the lady in the background with the tray".into(),
                "the coworker".into(),
                "she".into(),
                "her".into(),
            ],
            "to keep the room served and the work moving".into(),
        );
    }
    let (look, activity, position, alias) = match role {
        "scribe" => (
            "ink-stained sleeves",
            "copies a short note",
            "at the side desk",
            "the scribe",
        ),
        "reader" => (
            "a wool scarf",
            "turns a page by the window",
            "by the window",
            "the reader",
        ),
        "shop assistant" => (
            "a leather apron",
            "arranges small goods",
            "behind the counter",
            "the assistant",
        ),
        "customer" => (
            "a rain-dark cloak",
            "waits with a parcel",
            "near the entrance",
            "the customer",
        ),
        "adventurer" => (
            "a weathered traveling cloak with a scuffed pack",
            "examines a row of folded cloth",
            "at the shelves along the back wall",
            "the adventurer",
        ),
        "soldier" => (
            "a plain wool uniform beneath a repaired leather baldric",
            "keeps an eye on the shop door while waiting",
            "beside the front door",
            "the soldier",
        ),
        "dock worker" => (
            "a salt-stiff coat",
            "checks a coil of rope",
            "near the water side",
            "the dock worker",
        ),
        "boat hand" => (
            "a patched vest",
            "checks the mooring line",
            "beside the landing",
            "the boat hand",
        ),
        _ if index == 0 => (
            "a patched coat",
            "checks the nearby latch",
            "near the entrance",
            "the local worker",
        ),
        _ => (
            "a weathered pack",
            "watches the room quietly",
            "by the doorway",
            "the visitor",
        ),
    };
    (
        format!("a person with {look} {activity}"),
        position.into(),
        vec![alias.into(), role.into()],
        format!("to finish their work at {}", "this place"),
    )
}

#[derive(Clone, Copy)]
enum OwnerRef {
    None,
    Named(&'static str),
    Background(usize),
}

#[derive(Clone, Copy)]
struct ThingTemplate {
    name: &'static str,
    short: &'static str,
    detail: &'static str,
    picture: &'static str,
    closer: &'static str,
    skill: FindingSkill,
    dc: u8,
    portable: bool,
    value_cp: Option<u32>,
    use_effect: &'static str,
    position: &'static str,
    owner: OwnerRef,
    in_use: bool,
    aliases: &'static [&'static str],
}

const INN_THINGS: [ThingTemplate; 6] = [
    ThingTemplate {
        name: "dented pewter tankard",
        short: "a dented pewter tankard",
        detail: "The tankard is made of pewter, its rim flattened in one place and its handle polished by years of use. A faint smell of malt clings to the metal.",
        picture: "dented pewter mug on the bar",
        closer: "A thumb-sized dent marks the rim; this is the nearest mug on the bar.",
        skill: FindingSkill::Perception,
        dc: 10,
        portable: true,
        value_cp: None,
        use_effect: "Drink from it when it has ale, or set it down on the bar.",
        position: "on the bar in front",
        owner: OwnerRef::Named("patron"),
        in_use: false,
        aliases: &["mug", "tankard", "pewter mug", "pint", "on the bar"],
    },
    ThingTemplate {
        name: "carters' dice and cup",
        short: "bone dice and a carved wooden cup",
        detail: "The cup is turned from dark wood with a base worn smooth by repeated throws. Two yellowed bone dice lie inside, their black pips deeply cut and their corners rounded from many rolls.",
        picture: "bone dice in a wooden cup on the back table",
        closer: "A small notch on one die is only a wear mark; both dice are ordinary six-sided dice.",
        skill: FindingSkill::Investigation,
        dc: 10,
        portable: true,
        value_cp: None,
        use_effect: "Roll the dice from the cup as part of the carters' game.",
        position: "at the back table on the left",
        owner: OwnerRef::Background(0),
        in_use: true,
        aliases: &["dice cup", "cup", "wooden cup", "dice", "die", "bones"],
    },
    ThingTemplate {
        name: "dripping candle stub",
        short: "a dripping candle stub",
        detail: "The candle is a short twist of beeswax set in a shallow iron dish. Hardened drips ring the dish, and the warm wax smells faintly of honey.",
        picture: "dripping candle stub at the edge of the table",
        closer: "The wick is trimmed close and a little soot marks the dish above it.",
        skill: FindingSkill::Perception,
        dc: 10,
        portable: true,
        value_cp: None,
        use_effect: "Light it to add a small pool of light, or snuff it out.",
        position: "on the near table at the right",
        owner: OwnerRef::None,
        in_use: true,
        aliases: &["candle", "stub", "wax candle", "the candle"],
    },
    ThingTemplate {
        name: "bundle of drying herbs",
        short: "a bundle of drying herbs",
        detail: "Twine binds the stems into a small bundle hung from a dark beam. The leaves are brittle at their edges and smell sharply green when disturbed.",
        picture: "herbs tied to the beam above the bar",
        closer: "The stems are tied with the same blue thread used on the kitchen's herb jars.",
        skill: FindingSkill::Nature,
        dc: 10,
        portable: true,
        value_cp: None,
        use_effect: "Smell the leaves or take the bundle down if its owner agrees.",
        position: "above the bar on the left",
        owner: OwnerRef::Named("patron"),
        in_use: false,
        aliases: &["herbs", "herb bundle", "leaves"],
    },
    ThingTemplate {
        name: "stone hearth",
        short: "a low stone hearth",
        detail: "The hearth is built from soot-dark fieldstone with a broad ledge worn smooth by elbows. A steady fire warms the room and sends a woodsmoke smell toward the rafters.",
        picture: "lit stone hearth by the wall",
        closer: "A clean arc on the ledge shows where someone rested a kettle.",
        skill: FindingSkill::Perception,
        dc: 10,
        portable: false,
        value_cp: None,
        use_effect: "Sit nearby or warm your hands at the fire.",
        position: "by the hearth on the right",
        owner: OwnerRef::None,
        in_use: true,
        aliases: &["hearth", "fire", "fireplace"],
    },
    ThingTemplate {
        name: "brass wall clock",
        short: "a brass wall clock",
        detail: "The clock has a brass face behind cloudy glass and a narrow keyhole below the dial. Its slow tick carries across the quieter side of the room.",
        picture: "brass clock above the hearth",
        closer: "The hands show the hour; the minute hand shivers just before each tick.",
        skill: FindingSkill::Investigation,
        dc: 10,
        portable: false,
        value_cp: None,
        use_effect: "Read the time from its face.",
        position: "on the wall above the hearth",
        owner: OwnerRef::None,
        in_use: false,
        aliases: &["clock", "wall clock", "brass clock"],
    },
];

const READING_THINGS: [ThingTemplate; 6] = [
    ThingTemplate {
        name: "open ledger",
        short: "an open ledger",
        detail: "The ledger is bound in brown calfskin and held open by a brass weight. Its pages smell of dust and iron gall ink, with narrow columns ruled in blue.",
        picture: "open ledger on the reading table",
        closer: "A fresh line of ink is still glossy near the bottom of the page.",
        skill: FindingSkill::Investigation,
        dc: 10,
        portable: true,
        value_cp: None,
        use_effect: "Read the entries that are visible on the open page.",
        position: "on the table in front",
        owner: OwnerRef::None,
        in_use: false,
        aliases: &["ledger", "book", "open book"],
    },
    ThingTemplate {
        name: "green-shaded lamp",
        short: "a green-shaded lamp",
        detail: "The lamp has a heavy brass foot and a green glass shade that softens the light. A faint line of soot marks the chimney where the flame has burned high.",
        picture: "green-shaded lamp on the desk",
        closer: "The wick is nearly spent but the oil reservoir is still half full.",
        skill: FindingSkill::Perception,
        dc: 10,
        portable: true,
        value_cp: None,
        use_effect: "Turn the wick up or down to change the light.",
        position: "on the desk at the left",
        owner: OwnerRef::None,
        in_use: true,
        aliases: &["lamp", "green lamp", "light"],
    },
    ThingTemplate {
        name: "reading glass",
        short: "a round reading glass",
        detail: "The reading glass is set in a worn brass ring with a short walnut handle. Fingerprints cloud the edge of the lens but leave its center clear.",
        picture: "reading glass beside the ledger",
        closer: "A tiny maker's mark is stamped inside the brass ring.",
        skill: FindingSkill::Investigation,
        dc: 10,
        portable: true,
        value_cp: None,
        use_effect: "Magnify fine writing or small marks.",
        position: "beside the ledger in front",
        owner: OwnerRef::None,
        in_use: false,
        aliases: &["glass", "reading glass", "magnifier"],
    },
    ThingTemplate {
        name: "locked document case",
        short: "a locked document case",
        detail: "The case is made of dark oak with iron corners and a keyhole rubbed bright by use. It is heavier than its size suggests and gives a muted rattle when moved.",
        picture: "small iron-cornered case on the shelf",
        closer: "A paper fiber caught in the seam matches the rough stock used for the visible ledger.",
        skill: FindingSkill::Investigation,
        dc: 15,
        portable: true,
        value_cp: None,
        use_effect: "Open it with its key or another permitted method.",
        position: "on the shelf at the back",
        owner: OwnerRef::None,
        in_use: false,
        aliases: &["case", "document case", "box"],
    },
    ThingTemplate {
        name: "ink pot",
        short: "a lidded ink pot",
        detail: "The ink pot is glazed blue ceramic with a pewter lid. Its lower half is darkened by old spills and the lid smells sharply of ink.",
        picture: "blue ink pot near the papers",
        closer: "A dry crescent on the table shows where the pot was recently moved.",
        skill: FindingSkill::Perception,
        dc: 10,
        portable: true,
        value_cp: None,
        use_effect: "Lift the lid and inspect the ink.",
        position: "on the desk near the papers",
        owner: OwnerRef::None,
        in_use: false,
        aliases: &["ink", "ink pot", "pot"],
    },
    ThingTemplate {
        name: "tall window",
        short: "a tall leaded window",
        detail: "The window is set in a deep stone frame with leaded panes and a wooden latch. Beyond the glass, the weather dulls the view of the nearby street.",
        picture: "tall window along the far wall",
        closer: "One pane has a fresh scratch in the dust on its outer edge.",
        skill: FindingSkill::Perception,
        dc: 10,
        portable: false,
        value_cp: None,
        use_effect: "Look outside or open the latch if it is not secured.",
        position: "on the far wall to the right",
        owner: OwnerRef::None,
        in_use: false,
        aliases: &["window", "pane", "leaded window"],
    },
];

const SHOP_THINGS: [ThingTemplate; 6] = [
    ThingTemplate {
        name: "brass balance",
        short: "a small brass balance",
        detail: "The balance stands on a square oak base with brass pans hanging on fine chains. Its beam is scratched where weights have been set down for years.",
        picture: "brass balance on the counter",
        closer: "The left pan rocks once before settling level.",
        skill: FindingSkill::Investigation,
        dc: 10,
        portable: false,
        value_cp: None,
        use_effect: "Weigh a small item placed in either pan.",
        position: "on the counter in front",
        owner: OwnerRef::None,
        in_use: true,
        aliases: &["scale", "balance", "brass scale"],
    },
    ThingTemplate {
        name: "stoppered glass jar",
        short: "a stoppered glass jar",
        detail: "The jar is blown from green glass with a cork stopper and a cloudy ring near its base. It carries a faint smell of dried mint through the cork.",
        picture: "green glass jar on the shelf",
        closer: "A pale residue clings inside the neck below the stopper.",
        skill: FindingSkill::Nature,
        dc: 10,
        portable: true,
        value_cp: None,
        use_effect: "Open it to smell or inspect its contents.",
        position: "on the shelf at the back",
        owner: OwnerRef::None,
        in_use: false,
        aliases: &["jar", "glass jar", "bottle"],
    },
    ThingTemplate {
        name: "folded paper label",
        short: "a folded paper label",
        detail: "The label is thick rag paper folded around a small parcel. A faded red thread passes through a pinhole at its corner.",
        picture: "folded label beside the parcels",
        closer: "The crease hides a short line of writing on the inside.",
        skill: FindingSkill::Investigation,
        dc: 10,
        portable: true,
        value_cp: None,
        use_effect: "Unfold it and read the writing.",
        position: "beside the parcels on the counter",
        owner: OwnerRef::None,
        in_use: false,
        aliases: &["label", "paper", "folded note"],
    },
    ThingTemplate {
        name: "wooden counter",
        short: "a scarred wooden counter",
        detail: "The counter is made from thick oak boards darkened by polish and spilled oil. A shallow groove runs along the edge where small goods are set for customers.",
        picture: "scarred oak counter in front",
        closer: "A pale rectangle shows where a small object sat recently.",
        skill: FindingSkill::Perception,
        dc: 10,
        portable: false,
        value_cp: None,
        use_effect: "Set a small item down or lean against it.",
        position: "in front across the room",
        owner: OwnerRef::None,
        in_use: true,
        aliases: &["counter", "bar", "oak counter"],
    },
    ThingTemplate {
        name: "brass hand bell",
        short: "a brass hand bell",
        detail: "The bell has a narrow wooden handle and a brass cup rubbed bright at its lip. Its clear ring carries farther than its small size suggests.",
        picture: "hand bell near the counter edge",
        closer: "A thumbprint in the polish sits beside the handle's worn grip.",
        skill: FindingSkill::Perception,
        dc: 10,
        portable: true,
        value_cp: None,
        use_effect: "Ring it to call the attention of someone serving here.",
        position: "on the counter at the left",
        owner: OwnerRef::None,
        in_use: false,
        aliases: &["bell", "hand bell"],
    },
    ThingTemplate {
        name: "shuttered window",
        short: "a shuttered window",
        detail: "The window's pine shutters are swollen at the lower edge from damp. A narrow gap lets in the smell of rain and the sound of the street.",
        picture: "shuttered window on the left wall",
        closer: "The inside latch is bent slightly upward.",
        skill: FindingSkill::Investigation,
        dc: 10,
        portable: false,
        value_cp: None,
        use_effect: "Open the shutters to see outside.",
        position: "on the left wall",
        owner: OwnerRef::None,
        in_use: false,
        aliases: &["window", "shutters", "shuttered window"],
    },
];

const OUTDOOR_THINGS: [ThingTemplate; 6] = [
    ThingTemplate {
        name: "weathered signpost",
        short: "a weathered signpost",
        detail: "The signpost is a split oak trunk with two carved boards fixed by iron nails. Wind and rain have softened the painted lettering but left the cuts deep.",
        picture: "weathered signpost beside the path",
        closer: "One arrow has been turned toward the muddy track.",
        skill: FindingSkill::Survival,
        dc: 10,
        portable: false,
        value_cp: None,
        use_effect: "Read the directions or turn the loose arrow.",
        position: "beside the path on the left",
        owner: OwnerRef::None,
        in_use: false,
        aliases: &["signpost", "sign", "arrow"],
    },
    ThingTemplate {
        name: "iron lantern",
        short: "an iron lantern",
        detail: "The lantern's iron frame is blackened with soot and its horn panes are clouded. A small pool of old oil darkens the bottom of its glass reservoir.",
        picture: "iron lantern hanging from a post",
        closer: "The wick has been trimmed recently and the oil smells fresh.",
        skill: FindingSkill::Perception,
        dc: 10,
        portable: true,
        value_cp: None,
        use_effect: "Light it or carry it to brighten a nearby place.",
        position: "hanging beside the path",
        owner: OwnerRef::None,
        in_use: false,
        aliases: &["lantern", "light", "iron lamp"],
    },
    ThingTemplate {
        name: "stone marker",
        short: "a low stone marker",
        detail: "The marker is a flat gray stone set at the edge of the ground. Moss fills the shallow carving on its face, and rainwater beads along the top.",
        picture: "low stone marker in front",
        closer: "The carving is an old boundary mark rather than a name.",
        skill: FindingSkill::History,
        dc: 10,
        portable: false,
        value_cp: None,
        use_effect: "Read the mark or sit on the stone.",
        position: "in front at the path edge",
        owner: OwnerRef::None,
        in_use: false,
        aliases: &["marker", "stone", "boundary stone"],
    },
    ThingTemplate {
        name: "wooden water barrel",
        short: "a wooden water barrel",
        detail: "The barrel is bound with dark iron hoops and smells of damp oak. A tin dipper hangs from a leather loop at its side.",
        picture: "water barrel beside the wall",
        closer: "The water is clear, though a thin film of leaves floats near the edge.",
        skill: FindingSkill::Perception,
        dc: 10,
        portable: false,
        value_cp: None,
        use_effect: "Draw water with the tin dipper.",
        position: "against the wall on the right",
        owner: OwnerRef::None,
        in_use: false,
        aliases: &["barrel", "water", "water barrel"],
    },
    ThingTemplate {
        name: "low stone bench",
        short: "a low stone bench",
        detail: "The bench is a single slab of pale stone with a smooth seat and rough ends. Its surface is cool and flecked with grit from the weather.",
        picture: "stone bench by the path",
        closer: "A shallow groove runs along the seat where a strap has rubbed it.",
        skill: FindingSkill::Perception,
        dc: 10,
        portable: false,
        value_cp: None,
        use_effect: "Sit and rest for a moment.",
        position: "by the path on the right",
        owner: OwnerRef::None,
        in_use: false,
        aliases: &["bench", "stone seat"],
    },
    ThingTemplate {
        name: "wooden gate",
        short: "a wooden gate",
        detail: "The gate is built from rough pine rails and hangs on two black iron hinges. Its lower rail is scarred by carts and the latch is slick from rain.",
        picture: "wooden gate at the far path",
        closer: "Fresh mud on the latch shows it has been used today.",
        skill: FindingSkill::Investigation,
        dc: 10,
        portable: false,
        value_cp: None,
        use_effect: "Lift the latch and open the gate.",
        position: "at the far end of the path",
        owner: OwnerRef::None,
        in_use: false,
        aliases: &["gate", "wooden gate", "latch"],
    },
];

const GENERAL_THINGS: [ThingTemplate; 6] = [
    ThingTemplate {
        name: "scarred worktable",
        short: "a scarred worktable",
        detail: "The table is built from broad oak planks with one leg shimmed by a flat stone. Its surface is crosshatched with old knife marks and dark rings.",
        picture: "scarred oak table in front",
        closer: "A narrow groove runs under the near edge, too straight to be a natural split.",
        skill: FindingSkill::Investigation,
        dc: 10,
        portable: false,
        value_cp: None,
        use_effect: "Set down an item or work at the table.",
        position: "in front near the center",
        owner: OwnerRef::None,
        in_use: false,
        aliases: &["table", "worktable", "desk"],
    },
    ThingTemplate {
        name: "oil lantern",
        short: "a hooded oil lantern",
        detail: "The lantern has a black iron hood and a cloudy glass chimney. Warm oil and soot leave a faint smell around its handle.",
        picture: "hooded lantern on the shelf",
        closer: "The chimney is cracked near its base but still holds together.",
        skill: FindingSkill::Perception,
        dc: 10,
        portable: true,
        value_cp: None,
        use_effect: "Light it or carry it to brighten a nearby place.",
        position: "on the shelf at the left",
        owner: OwnerRef::None,
        in_use: false,
        aliases: &["lantern", "lamp", "light"],
    },
    ThingTemplate {
        name: "small iron key",
        short: "a small iron key",
        detail: "The key is dark iron with a square bow and a long narrow bit. Its teeth are worn smooth and the metal has a cold, oily feel.",
        picture: "small key beside the papers",
        closer: "A thread of red wool is caught in the bow.",
        skill: FindingSkill::Investigation,
        dc: 10,
        portable: true,
        value_cp: None,
        use_effect: "Try it in a lock of a matching size.",
        position: "beside the papers on the right",
        owner: OwnerRef::None,
        in_use: false,
        aliases: &["key", "iron key"],
    },
    ThingTemplate {
        name: "lidded storage chest",
        short: "a lidded storage chest",
        detail: "The chest is made from rough pine boards with leather straps at its corners. The lid creaks on two iron hinges and carries the smell of cedar shavings.",
        picture: "lidded chest against the wall",
        closer: "The latch has no lock, though the hasp has been bent once.",
        skill: FindingSkill::Investigation,
        dc: 10,
        portable: false,
        value_cp: None,
        use_effect: "Lift the lid and look inside.",
        position: "against the back wall",
        owner: OwnerRef::None,
        in_use: false,
        aliases: &["chest", "box", "storage chest"],
    },
    ThingTemplate {
        name: "clay cup",
        short: "a plain clay cup",
        detail: "The cup is fired red clay with a small chip in its lip. A dull glaze covers the outside but leaves the base rough to the touch.",
        picture: "clay cup on the table",
        closer: "A trace of clean water remains at the bottom.",
        skill: FindingSkill::Perception,
        dc: 10,
        portable: true,
        value_cp: None,
        use_effect: "Drink from it or set it down.",
        position: "on the table in front",
        owner: OwnerRef::None,
        in_use: false,
        aliases: &["cup", "mug", "clay cup"],
    },
    ThingTemplate {
        name: "wooden window shutter",
        short: "a wooden window shutter",
        detail: "The shutter is made from narrow pine boards darkened by years of weather. Its iron hook is cool and leaves rust on the fingers.",
        picture: "shuttered window along the wall",
        closer: "A thin stripe of daylight shows under the edge.",
        skill: FindingSkill::Perception,
        dc: 10,
        portable: false,
        value_cp: None,
        use_effect: "Open or close it to change the light and view.",
        position: "on the wall at the left",
        owner: OwnerRef::None,
        in_use: false,
        aliases: &["window", "shutter", "shutters"],
    },
];

fn things(
    setting: &SceneSetting,
    place: ScenePlace,
    people: &[ScenePerson],
) -> Result<Vec<SceneThing>, SceneModelError> {
    let templates = match setting.kind {
        SettingKind::Inn | SettingKind::TeaHouse => &INN_THINGS,
        SettingKind::Archive
        | SettingKind::LibraryHall
        | SettingKind::ArchivistsStudy
        | SettingKind::ReadingGallery
        | SettingKind::MapRoom => &READING_THINGS,
        SettingKind::Shop | SettingKind::Apothecary | SettingKind::LanternShop => &SHOP_THINGS,
        SettingKind::Sheepfold
        | SettingKind::Quay
        | SettingKind::ForestTrail
        | SettingKind::MountainPass
        | SettingKind::MarketStreet
        | SettingKind::GraveyardGate
        | SettingKind::MarshBoardwalk
        | SettingKind::Farmyard
        | SettingKind::LibraryForecourt => &OUTDOOR_THINGS,
        _ => &GENERAL_THINGS,
    };
    let background_ids = people
        .iter()
        .filter(|person| person.is_background)
        .map(|person| person.id.as_str())
        .collect::<Vec<_>>();
    templates
        .iter()
        .enumerate()
        .map(|(index, template)| {
            let owner_id = match template.owner {
                OwnerRef::None => None,
                OwnerRef::Named(id) => people
                    .iter()
                    .any(|person| person.id == id)
                    .then(|| id.to_owned()),
                OwnerRef::Background(owner_index) => background_ids
                    .get(owner_index)
                    .map(|owner| (*owner).to_owned()),
            };
            let target_findings = place == ScenePlace::Meeting && setting.kind == SettingKind::Inn;
            let (finding_id, clue_fact_id) = match (target_findings, index) {
                (true, 0) => (Some("errand-truth".into()), Some("secret.id".into())),
                (true, 1) => (Some("enemy-position".into()), Some("ant.id".into())),
                _ => (None, None),
            };
            let mut aliases = template
                .aliases
                .iter()
                .map(|alias| (*alias).to_owned())
                .collect::<Vec<_>>();
            aliases.push(template.name.to_owned());
            Ok(SceneThing {
                id: format!("{}-thing-{}", setting.place_id, index + 1),
                name: template.name.to_owned(),
                short: template.short.to_owned(),
                detail: template.detail.to_owned(),
                picture_description: template.picture.to_owned(),
                closer_look: template.closer.to_owned(),
                skill: template.skill,
                dc: if index == 0 { 10 } else { template.dc },
                finding_id,
                clue_fact_id,
                portable: template.portable,
                value_cp: template.value_cp,
                use_effect: template.use_effect.to_owned(),
                position: template.position.to_owned(),
                owner_id,
                in_use: template.in_use,
                in_scene: true,
                publicly_visible: true,
                holder_id: None,
                open: template.name.starts_with("open "),
                lit: template.name.contains("hearth")
                    || template.name.contains("lantern")
                    || template.name.contains("candle"),
                broken: false,
                searched_by: Vec::new(),
                aliases: {
                    aliases.push(template.position.to_owned());
                    std::mem::take(&mut aliases)
                },
                provenance: SceneItemProvenance {
                    kind: "authored_location".into(),
                    source_id: format!("{}-thing-{}", setting.place_id, index + 1),
                    seller_id: None,
                },
            })
        })
        .collect()
}

fn ways_out(setting: &SceneSetting, time: &str, weather: &str) -> Vec<SceneWayOut> {
    let beyond = format!(
        "{} in {time}; {weather} is visible beyond it",
        setting.short
    );
    if setting.exterior {
        vec![
            SceneWayOut {
                id: format!("{}-way-road", setting.place_id),
                name: "road out".into(),
                aliases: vec!["road", "path out", "track"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                position: "ahead to the right".into(),
                beyond_place_id: "none".into(),
                beyond: beyond.clone(),
                open: true,
            },
            SceneWayOut {
                id: format!("{}-way-return", setting.place_id),
                name: "way back".into(),
                aliases: vec!["way back", "path behind", "return path"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                position: "behind the party".into(),
                beyond_place_id: "none".into(),
                beyond,
                open: true,
            },
        ]
    } else {
        vec![
            SceneWayOut {
                id: format!("{}-way-front-door", setting.place_id),
                name: "front door".into(),
                aliases: vec!["door", "front door", "entrance"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                position: "at the front on the right".into(),
                beyond_place_id: "none".into(),
                beyond: beyond.clone(),
                open: false,
            },
            SceneWayOut {
                id: format!("{}-way-window", setting.place_id),
                name: "window".into(),
                aliases: vec!["window", "outside", "street"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                position: "on the left wall".into(),
                beyond_place_id: "none".into(),
                beyond,
                open: false,
            },
            SceneWayOut {
                id: format!("{}-way-side-stair", setting.place_id),
                name: "side stair".into(),
                aliases: vec!["stair", "stairs", "side stair"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                position: "at the back on the left".into(),
                beyond_place_id: "none".into(),
                beyond: format!(
                    "a dim passage continues beyond the stair from {}",
                    setting.name
                ),
                open: true,
            },
        ]
    }
}

fn offers(setting: &SceneSetting, people: &[ScenePerson]) -> Vec<SceneOffer> {
    let seller_id = people
        .iter()
        .find(|person| person.id == "patron" || person.id == "contact")
        .or_else(|| people.iter().find(|person| person.role == "serving woman"))
        .map(|person| person.id.clone());
    let Some(seller_id) = seller_id else {
        return Vec::new();
    };
    if setting.kind == SettingKind::Shop {
        return vec![
            offer(
                "clothes-travelers",
                &seller_id,
                "traveler's clothes",
                &["traveler's clothes", "travel clothes", "clothes"],
                "a practical set of traveler's clothes for the road",
                200,
            ),
            offer(
                "clothes-fine",
                &seller_id,
                "fine clothes",
                &["fine clothes", "fine clothing", "formal clothes"],
                "a well-made set of fine clothes",
                1500,
            ),
            offer(
                "costume",
                &seller_id,
                "costume",
                &["costume", "disguise", "theatrical costume"],
                "a complete costume suitable for a disguise or performance",
                500,
            ),
        ];
    }
    if !matches!(setting.kind, SettingKind::Inn | SettingKind::TeaHouse) {
        return Vec::new();
    }
    vec![
        offer(
            "ale-mug",
            &seller_id,
            "ale (mug)",
            &["ale", "beer", "pint"],
            "a mug of ale",
            4,
        ),
        offer(
            "bread-loaf",
            &seller_id,
            "bread (loaf)",
            &["bread", "loaf"],
            "a loaf of bread",
            2,
        ),
        offer(
            "poor-meal",
            &seller_id,
            "simple meal",
            &["meal", "food", "stew"],
            "a simple meal at the poor lifestyle price",
            2,
        ),
        offer(
            "cheese-wedge",
            &seller_id,
            "cheese (wedge)",
            &["cheese", "wedge"],
            "a wedge of cheese",
            10,
        ),
        offer(
            "poor-inn-stay",
            &seller_id,
            "one night's poor lodging",
            &["bed", "room", "lodging", "stay the night"],
            "one night's lodging at the poor lifestyle price",
            10,
        ),
    ]
}

fn offer(
    id: &str,
    seller_id: &str,
    name: &str,
    aliases: &[&str],
    description: &str,
    cost_cp: u32,
) -> SceneOffer {
    SceneOffer {
        id: id.into(),
        item_id: format!("item-{id}"),
        seller_id: seller_id.into(),
        name: name.into(),
        aliases: aliases.iter().map(|alias| (*alias).into()).collect(),
        description: description.into(),
        cost_cp,
    }
}

fn scene_conditions(setting: &SceneSetting, seed: u64) -> (String, String) {
    let conditions = [
        ("morning", "clear cool air"),
        ("afternoon", "a light rain"),
        ("dusk", "a low mist"),
        ("night", "a steady wind"),
    ];
    let (time, weather) = conditions[stable_offset(seed, &setting.place_id) % conditions.len()];
    let weather = if setting.exterior {
        weather.to_owned()
    } else {
        format!("{weather} outside")
    };
    (time.into(), weather)
}

fn stable_offset(seed: u64, place_id: &str) -> usize {
    let hash = place_id
        .bytes()
        .fold(seed ^ 0xcbf2_9ce4_8422_2325, |state, byte| {
            state
                .wrapping_mul(0x100_0000_01b3)
                .wrapping_add(u64::from(byte))
        });
    hash as usize
}

fn required<'a>(
    facts: &'a BTreeMap<String, String>,
    key: &str,
) -> Result<&'a str, SceneModelError> {
    facts
        .get(key)
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or(SceneModelError::MissingFact)
}

fn valid_text(value: &str, maximum: usize) -> bool {
    !value.trim().is_empty() && value.chars().count() <= maximum
}

fn sentence_count(value: &str) -> usize {
    value
        .split(['.', '!', '?'])
        .filter(|sentence| !sentence.trim().is_empty())
        .count()
}

fn scene_key_label(key: SceneKey) -> &'static str {
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

fn normalize(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    let mut space = true;
    for character in value.chars().flat_map(char::to_lowercase) {
        if character.is_alphanumeric() {
            normalized.push(character);
            space = false;
        } else if !space {
            normalized.push(' ');
            space = true;
        }
    }
    normalized.trim().to_owned()
}

fn contains_phrase(text: &str, phrase: &str) -> bool {
    let phrase = normalize(phrase);
    if phrase.is_empty() {
        return false;
    }
    format!(" {text} ").contains(&format!(" {phrase} "))
}

fn best_match<'a, T>(
    entries: &'a [T],
    line: &str,
    aliases: impl Fn(&'a T) -> &'a [String],
) -> Option<&'a T> {
    entries
        .iter()
        .filter_map(|entry| {
            aliases(entry)
                .iter()
                .map(|alias| normalize(alias))
                .filter(|alias| contains_phrase(line, alias))
                .max_by_key(String::len)
                .map(|alias| (entry, alias.len()))
        })
        .max_by_key(|(_, length)| *length)
        .map(|(entry, _)| entry)
}

fn best_visible_thing<'a>(things: &'a [SceneThing], line: &str) -> Option<&'a SceneThing> {
    things
        .iter()
        .filter(|thing| thing.in_scene)
        .filter_map(|thing| {
            thing
                .aliases
                .iter()
                .map(|alias| normalize(alias))
                .filter(|alias| contains_phrase(line, alias))
                .max_by_key(String::len)
                .map(|alias| (thing, alias.len()))
        })
        .max_by_key(|(_, length)| *length)
        .map(|(thing, _)| thing)
}

fn matching_people<'a>(people: &'a [ScenePerson], line: &str) -> Vec<&'a ScenePerson> {
    let matched_aliases = people
        .iter()
        .flat_map(|person| {
            person
                .aliases
                .iter()
                .map(|alias| normalize(alias))
                .filter(|alias| contains_phrase(line, alias))
        })
        .collect::<Vec<_>>();
    let Some(longest) = matched_aliases.iter().map(String::len).max() else {
        return Vec::new();
    };
    let aliases = matched_aliases
        .into_iter()
        .filter(|alias| alias.len() == longest)
        .collect::<BTreeSet<_>>();
    people
        .iter()
        .filter(|person| {
            person
                .aliases
                .iter()
                .map(|alias| normalize(alias))
                .any(|alias| aliases.contains(&alias))
        })
        .collect()
}
